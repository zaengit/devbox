use crate::model::ManagedServiceProfile;
use anyhow::{Context, Result, anyhow};
use futures::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpStream;
use tokio::process::Command;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ServiceSnapshot {
    pub id: String,
    pub name: String,
    pub category: String,
    pub executable_path: Option<String>,
    pub discovery_source: String,
    pub process_count: usize,
    pub endpoint_ports: Vec<u16>,
    pub reachable_port: Option<u16>,
    pub state: ServiceState,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ServiceState {
    Running,
    EndpointReachable,
    InstalledStopped,
    Unhealthy,
    Missing,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ManagedServiceSnapshot {
    pub service_id: String,
    pub name: String,
    pub pid: u32,
    pub status: String,
    pub started_at: i64,
    pub health: String,
    pub logs: VecDeque<String>,
}

#[derive(Clone, Default)]
pub struct ServiceManager {
    states: Arc<Mutex<HashMap<String, ManagedServiceSnapshot>>>,
}

impl ServiceManager {
    pub fn snapshots(&self) -> Vec<ManagedServiceSnapshot> {
        self.states
            .lock()
            .map(|states| states.values().cloned().collect())
            .unwrap_or_default()
    }

    pub async fn start(&self, profile: &ManagedServiceProfile) -> Result<ManagedServiceSnapshot> {
        if self
            .states
            .lock()
            .map_err(|_| anyhow!("service state lock poisoned"))?
            .get(&profile.id)
            .is_some_and(|state| state.status == "running" || state.status == "stopping")
        {
            return Err(anyhow!("this Devbox-managed service is already running"));
        }
        for port in &profile.ports {
            if *port < 1024 {
                return Err(anyhow!(
                    "service ports below 1024 are not allowed; select an unprivileged port"
                ));
            }
            if port_is_reachable(*port).await {
                return Err(anyhow!(
                    "port {port} is already in use by a process Devbox does not own"
                ));
            }
        }
        let executable = crate::validate_executable(Path::new(&profile.executable_path))
            .with_context(|| {
                format!("validating service executable {}", profile.executable_path)
            })?;
        let mut command = Command::new(&executable);
        command
            .args(&profile.arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(directory) = &profile.data_directory {
            let data_directory = validate_service_data_directory(Path::new(directory))?;
            command.current_dir(data_directory);
        }
        crate::platform::configure_owned_process(&mut command);
        let mut child = command
            .spawn()
            .with_context(|| format!("starting service {}", profile.name))?;
        let pid = child
            .id()
            .ok_or_else(|| anyhow!("operating system did not return a service process ID"))?;
        let snapshot = ManagedServiceSnapshot {
            service_id: profile.id.clone(),
            name: profile.name.clone(),
            pid,
            status: "running".into(),
            started_at: crate::model::now_unix(),
            health: if profile.ports.is_empty() {
                "process running; no health port configured".into()
            } else {
                "starting; checking configured TCP ports".into()
            },
            logs: VecDeque::new(),
        };
        self.states
            .lock()
            .map_err(|_| anyhow!("service state lock poisoned"))?
            .insert(profile.id.clone(), snapshot.clone());
        if let Some(stdout) = child.stdout.take() {
            self.spawn_log_reader(profile.id.clone(), "stdout", stdout);
        }
        if let Some(stderr) = child.stderr.take() {
            self.spawn_log_reader(profile.id.clone(), "stderr", stderr);
        }
        let states = Arc::clone(&self.states);
        let id = profile.id.clone();
        let ports = profile.ports.clone();
        tokio::spawn(async move {
            let result = child.wait().await;
            // Keep a daemonizing or shell wrapper from leaving unmanaged descendants behind.
            let _ = crate::platform::stop_owned_process_group(
                pid,
                Duration::from_millis(300),
                "service child",
            )
            .await;
            match result {
                Ok(exit) => {
                    if let Ok(mut states) = states.lock()
                        && let Some(state) = states.get_mut(&id)
                    {
                        state.status = if exit.success() { "exited" } else { "failed" }.into();
                        state.health = format!("process exited with {exit}");
                    }
                }
                Err(error) => {
                    if let Ok(mut states) = states.lock()
                        && let Some(state) = states.get_mut(&id)
                    {
                        state.status = "failed".into();
                        state.health = format!("failed waiting for process: {error}");
                    }
                }
            }
        });
        let states = Arc::clone(&self.states);
        let id = profile.id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(350)).await;
            let mut reachable = Vec::new();
            for port in ports {
                if port_is_reachable(port).await {
                    reachable.push(port);
                }
            }
            if let Ok(mut states) = states.lock()
                && let Some(state) = states.get_mut(&id)
                && state.status == "running"
            {
                state.health = if reachable.is_empty() && !state.health.contains("no health port") {
                    "process running; configured ports are not responding yet".into()
                } else if reachable.is_empty() {
                    state.health.clone()
                } else {
                    format!(
                        "healthy on TCP {}",
                        reachable
                            .iter()
                            .map(u16::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
            }
        });
        Ok(snapshot)
    }

    pub async fn wait_until_ready(&self, service_id: &str, ports: &[u16]) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let snapshot = self
                .states
                .lock()
                .map_err(|_| anyhow!("service state lock poisoned"))?
                .get(service_id)
                .cloned()
                .ok_or_else(|| anyhow!("no Devbox-owned process is recorded for this service"))?;
            if snapshot.status != "running" {
                return Err(anyhow!(
                    "service {} exited before becoming ready: {}",
                    snapshot.name,
                    snapshot.health
                ));
            }
            if ports.is_empty() {
                if tokio::time::Instant::now() + Duration::from_millis(150) >= deadline {
                    return Err(anyhow!("service readiness deadline elapsed"));
                }
                tokio::time::sleep(Duration::from_millis(150)).await;
                let running = self
                    .states
                    .lock()
                    .ok()
                    .and_then(|states| {
                        states
                            .get(service_id)
                            .map(|state| state.status == "running")
                    })
                    .unwrap_or(false);
                if running {
                    return Ok(());
                }
            } else {
                let mut available = Vec::new();
                for port in ports {
                    if port_is_reachable(*port).await {
                        available.push(*port);
                    }
                }
                if available.len() == ports.len() {
                    return Ok(());
                }
                if tokio::time::Instant::now() >= deadline {
                    let missing = ports
                        .iter()
                        .filter(|port| !available.contains(port))
                        .map(u16::to_string)
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Err(anyhow!(
                        "service {} did not become ready on TCP port(s) {missing} within 10 seconds",
                        snapshot.name
                    ));
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!("service did not become ready within 10 seconds"));
            }
        }
    }

    pub async fn stop(&self, service_id: &str) -> Result<()> {
        let (pid, started_at) = {
            let mut states = self
                .states
                .lock()
                .map_err(|_| anyhow!("service state lock poisoned"))?;
            let state = states
                .get_mut(service_id)
                .ok_or_else(|| anyhow!("no Devbox-owned process is recorded for this service"))?;
            if state.status != "running" && state.status != "stopping" {
                return Err(anyhow!("the Devbox-managed service is not running"));
            }
            state.status = "stopping".into();
            (state.pid, state.started_at)
        };
        if let Err(error) = crate::platform::stop_owned_process_group(
            pid,
            Duration::from_secs(4),
            "Devbox-owned service",
        )
        .await
        {
            self.restore_running(service_id, started_at);
            return Err(error);
        }
        Ok(())
    }

    fn restore_running(&self, service_id: &str, started_at: i64) {
        if let Ok(mut states) = self.states.lock()
            && let Some(state) = states.get_mut(service_id)
            && state.started_at == started_at
        {
            state.status = "running".into();
        }
    }

    fn spawn_log_reader<R>(&self, id: String, stream: &'static str, output: R)
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
    {
        let states = Arc::clone(&self.states);
        tokio::spawn(async move {
            let mut lines = BufReader::new(output).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(mut states) = states.lock()
                    && let Some(state) = states.get_mut(&id)
                {
                    if state.logs.len() == 300 {
                        state.logs.pop_front();
                    }
                    state.logs.push_back(format!(
                        "[{stream}] {}",
                        line.chars().take(4000).collect::<String>()
                    ));
                }
            }
        });
    }
}

fn validate_service_data_directory(path: &Path) -> Result<PathBuf> {
    let canonical = path
        .canonicalize()
        .with_context(|| format!("resolving service data directory {}", path.display()))?;
    if !canonical.is_dir() {
        return Err(anyhow!("service data path must be a directory"));
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::metadata(&canonical)?;
        // Do not allow a Devbox-managed process to write into a directory owned by another user.
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err(anyhow!(
                "service data directory must be owned by the current user"
            ));
        }
    }
    Ok(canonical)
}

async fn port_is_reachable(port: u16) -> bool {
    tokio::time::timeout(
        Duration::from_millis(100),
        TcpStream::connect(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)),
    )
    .await
    .is_ok_and(|result| result.is_ok())
}

#[derive(Clone)]
struct ServiceDefinition {
    id: &'static str,
    name: &'static str,
    category: &'static str,
    binaries: &'static [&'static str],
    processes: &'static [&'static str],
    ports: &'static [u16],
}

const SERVICES: &[ServiceDefinition] = &[
    ServiceDefinition {
        id: "mysql",
        name: "MySQL / MariaDB",
        category: "Database",
        binaries: &["mysqld", "mariadbd"],
        processes: &["mysqld", "mariadbd"],
        ports: &[3306],
    },
    ServiceDefinition {
        id: "postgresql",
        name: "PostgreSQL",
        category: "Database",
        binaries: &["postgres"],
        processes: &["postgres", "postmaster"],
        ports: &[5432],
    },
    ServiceDefinition {
        id: "mongodb",
        name: "MongoDB",
        category: "Database",
        binaries: &["mongod"],
        processes: &["mongod"],
        ports: &[27017],
    },
    ServiceDefinition {
        id: "redis",
        name: "Redis",
        category: "Cache",
        binaries: &["redis-server"],
        processes: &["redis-server"],
        ports: &[6379],
    },
    ServiceDefinition {
        id: "valkey",
        name: "Valkey",
        category: "Cache",
        binaries: &["valkey-server"],
        processes: &["valkey-server"],
        ports: &[6379],
    },
    ServiceDefinition {
        id: "mailpit",
        name: "Mailpit",
        category: "Email",
        binaries: &["mailpit"],
        processes: &["mailpit"],
        ports: &[1025, 8025],
    },
    ServiceDefinition {
        id: "garage",
        name: "Garage S3",
        category: "Object storage",
        binaries: &["garage"],
        processes: &["garage"],
        ports: &[3900],
    },
    ServiceDefinition {
        id: "caddy",
        name: "Caddy",
        category: "Web server",
        binaries: &["caddy"],
        processes: &["caddy"],
        ports: &[80, 443],
    },
    ServiceDefinition {
        id: "nginx",
        name: "Nginx",
        category: "Web server",
        binaries: &["nginx"],
        processes: &["nginx"],
        ports: &[80, 443],
    },
    ServiceDefinition {
        id: "apache",
        name: "Apache HTTP Server",
        category: "Web server",
        binaries: &["apache2", "httpd"],
        processes: &["apache2", "httpd"],
        ports: &[80, 443],
    },
];

#[derive(Default)]
struct StaticDiscovery {
    paths: HashMap<&'static str, (String, String)>,
    processes: HashMap<&'static str, usize>,
}

pub async fn scan_services() -> Vec<ServiceSnapshot> {
    let discovered = tokio::task::spawn_blocking(discover_static)
        .await
        .unwrap_or_default();
    let discovered = Arc::new(discovered);
    let definitions = SERVICES.to_vec();
    stream::iter(definitions.into_iter().map(|definition| {
        let discovered = Arc::clone(&discovered);
        async move {
            let reachable_port = stream::iter(definition.ports.iter().copied())
                .map(|port| async move {
                    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
                    match tokio::time::timeout(
                        Duration::from_millis(180),
                        TcpStream::connect(address),
                    )
                    .await
                    {
                        Ok(Ok(_)) => Some(port),
                        _ => None,
                    }
                })
                .buffer_unordered(definition.ports.len().max(1))
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .flatten()
                .next();
            let (executable_path, discovery_source) =
                discovered.paths.get(definition.id).cloned().unwrap_or((
                    String::new(),
                    "Not found in known binary directories".into(),
                ));
            let process_count = discovered
                .processes
                .get(definition.id)
                .copied()
                .unwrap_or_default();
            let state = if process_count > 0 {
                if reachable_port.is_some() || definition.ports.is_empty() {
                    ServiceState::Running
                } else {
                    ServiceState::Unhealthy
                }
            } else if reachable_port.is_some() {
                // A listener may be an externally managed service even when its server binary is not in PATH.
                ServiceState::EndpointReachable
            } else if executable_path.is_empty() {
                ServiceState::Missing
            } else {
                ServiceState::InstalledStopped
            };
            ServiceSnapshot {
                id: definition.id.into(),
                name: definition.name.into(),
                category: definition.category.into(),
                executable_path: (!executable_path.is_empty()).then_some(executable_path),
                discovery_source,
                process_count,
                endpoint_ports: definition.ports.to_vec(),
                reachable_port,
                state,
            }
        }
    }))
    .buffer_unordered(8)
    .collect()
    .await
}

fn discover_static() -> StaticDiscovery {
    let path_directories = std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default();
    let path_set = path_directories.iter().cloned().collect::<HashSet<_>>();
    let mut directories = path_directories;
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        directories.extend([
            home.join(".local/bin"),
            home.join(".cargo/bin"),
            home.join(".local/share/mise/shims"),
        ]);
    }
    directories.extend([
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
        PathBuf::from("/usr/sbin"),
        PathBuf::from("/sbin"),
    ]);

    let mut result = StaticDiscovery::default();
    for directory in directories {
        for definition in SERVICES {
            for binary in definition.binaries {
                let candidate = directory.join(binary);
                let Ok(metadata) = std::fs::metadata(&candidate) else {
                    continue;
                };
                if !metadata.is_file() {
                    continue;
                }
                let Ok(resolved) = candidate.canonicalize() else {
                    continue;
                };
                let source = if path_set.contains(&directory) {
                    "PATH"
                } else {
                    "Known system/user directory"
                };
                result
                    .paths
                    .entry(definition.id)
                    .or_insert((resolved.display().to_string(), source.into()));
            }
        }
    }

    #[cfg(target_os = "linux")]
    if let Ok(entries) = std::fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            if !file_name
                .to_string_lossy()
                .bytes()
                .all(|byte| byte.is_ascii_digit())
            {
                continue;
            }
            let Ok(command) = std::fs::read_to_string(entry.path().join("comm")) else {
                continue;
            };
            let command = command.trim();
            for definition in SERVICES {
                if definition.processes.contains(&command) {
                    *result.processes.entry(definition.id).or_default() += 1;
                }
            }
        }
    }
    result
}
