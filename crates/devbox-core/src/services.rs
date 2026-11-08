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
    #[serde(default)]
    pub systemd_unit: Option<String>,
    #[serde(default)]
    pub systemd_active_state: Option<String>,
    #[serde(default)]
    pub systemd_sub_state: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostServiceAction {
    Start,
    Stop,
    Restart,
}

impl HostServiceAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::Stop => "Stop",
            Self::Restart => "Restart",
        }
    }

    fn systemctl_verb(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
        }
    }
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
    systemd_units: &'static [&'static str],
}

const SERVICES: &[ServiceDefinition] = &[
    ServiceDefinition {
        id: "mysql",
        name: "MySQL / MariaDB",
        category: "Database",
        binaries: &["mysqld", "mariadbd"],
        processes: &["mysqld", "mariadbd"],
        ports: &[3306],
        systemd_units: &["mariadb.service", "mysql.service"],
    },
    ServiceDefinition {
        id: "postgresql",
        name: "PostgreSQL",
        category: "Database",
        binaries: &["postgres"],
        processes: &["postgres", "postmaster"],
        ports: &[5432],
        systemd_units: &["postgresql.service"],
    },
    ServiceDefinition {
        id: "mongodb",
        name: "MongoDB",
        category: "Database",
        binaries: &["mongod"],
        processes: &["mongod"],
        ports: &[27017],
        systemd_units: &["mongod.service"],
    },
    ServiceDefinition {
        id: "clickhouse",
        name: "ClickHouse",
        category: "Database",
        binaries: &["clickhouse-server"],
        // Linux truncates the process comm field to 15 bytes.
        processes: &["clickhouse-serv"],
        ports: &[8123],
        systemd_units: &["clickhouse-server.service"],
    },
    ServiceDefinition {
        id: "cockroachdb",
        name: "CockroachDB",
        category: "Database",
        binaries: &["cockroach"],
        processes: &["cockroach"],
        ports: &[26257],
        systemd_units: &["cockroach.service", "cockroachdb.service"],
    },
    ServiceDefinition {
        id: "mssql",
        name: "Microsoft SQL Server",
        category: "Database",
        binaries: &["sqlservr"],
        processes: &["sqlservr"],
        ports: &[1433],
        systemd_units: &["mssql-server.service"],
    },
    ServiceDefinition {
        id: "couchdb",
        name: "CouchDB",
        category: "Database",
        binaries: &["couchdb"],
        processes: &[],
        ports: &[5984],
        systemd_units: &["couchdb.service"],
    },
    ServiceDefinition {
        id: "neo4j",
        name: "Neo4j",
        category: "Database",
        binaries: &["neo4j"],
        processes: &[],
        ports: &[7474, 7687],
        systemd_units: &["neo4j.service"],
    },
    ServiceDefinition {
        id: "etcd",
        name: "etcd",
        category: "Service discovery",
        binaries: &["etcd"],
        processes: &["etcd"],
        ports: &[2379],
        systemd_units: &["etcd.service"],
    },
    ServiceDefinition {
        id: "consul",
        name: "Consul",
        category: "Service discovery",
        binaries: &["consul"],
        processes: &["consul"],
        ports: &[8500],
        systemd_units: &["consul.service"],
    },
    ServiceDefinition {
        id: "zookeeper",
        name: "ZooKeeper",
        category: "Message broker",
        binaries: &["zkServer.sh"],
        // ZooKeeper runs on Java; matching the generic JVM process would be misleading.
        processes: &[],
        ports: &[2181],
        systemd_units: &["zookeeper.service"],
    },
    ServiceDefinition {
        id: "redis",
        name: "Redis",
        category: "Cache",
        binaries: &["redis-server"],
        processes: &["redis-server"],
        ports: &[6379],
        systemd_units: &["redis-server.service", "redis.service"],
    },
    ServiceDefinition {
        id: "valkey",
        name: "Valkey",
        category: "Cache",
        binaries: &["valkey-server"],
        processes: &["valkey-server"],
        ports: &[6379],
        systemd_units: &["valkey-server.service", "valkey.service"],
    },
    ServiceDefinition {
        id: "memcached",
        name: "Memcached",
        category: "Cache",
        binaries: &["memcached"],
        processes: &["memcached"],
        ports: &[11211],
        systemd_units: &["memcached.service"],
    },
    ServiceDefinition {
        id: "varnish",
        name: "Varnish",
        category: "Cache",
        binaries: &["varnishd"],
        processes: &["varnishd"],
        ports: &[6081],
        systemd_units: &["varnish.service"],
    },
    ServiceDefinition {
        id: "rabbitmq",
        name: "RabbitMQ",
        category: "Message broker",
        binaries: &["rabbitmq-server"],
        // The Erlang VM process name is shared by unrelated applications.
        processes: &[],
        ports: &[5672],
        systemd_units: &["rabbitmq-server.service"],
    },
    ServiceDefinition {
        id: "nats",
        name: "NATS",
        category: "Message broker",
        binaries: &["nats-server"],
        processes: &["nats-server"],
        ports: &[4222],
        systemd_units: &["nats-server.service", "nats.service"],
    },
    ServiceDefinition {
        id: "kafka",
        name: "Apache Kafka",
        category: "Message broker",
        binaries: &["kafka-server-start.sh"],
        // Kafka runs on Java; matching the generic JVM process would be misleading.
        processes: &[],
        ports: &[9092],
        systemd_units: &["kafka.service", "confluent-kafka.service"],
    },
    ServiceDefinition {
        id: "elasticsearch",
        name: "Elasticsearch",
        category: "Search",
        binaries: &["elasticsearch"],
        processes: &["elasticsearch"],
        ports: &[9200],
        systemd_units: &["elasticsearch.service"],
    },
    ServiceDefinition {
        id: "opensearch",
        name: "OpenSearch",
        category: "Search",
        binaries: &["opensearch"],
        // OpenSearch also runs on Java; avoid treating every JVM as this service.
        processes: &[],
        ports: &[9200],
        systemd_units: &["opensearch.service"],
    },
    ServiceDefinition {
        id: "meilisearch",
        name: "Meilisearch",
        category: "Search",
        binaries: &["meilisearch"],
        processes: &["meilisearch"],
        ports: &[7700],
        systemd_units: &["meilisearch.service"],
    },
    ServiceDefinition {
        id: "typesense",
        name: "Typesense",
        category: "Search",
        binaries: &["typesense-server"],
        processes: &["typesense-serve"],
        ports: &[8108],
        systemd_units: &["typesense-server.service", "typesense.service"],
    },
    ServiceDefinition {
        id: "qdrant",
        name: "Qdrant",
        category: "Search",
        binaries: &["qdrant"],
        processes: &["qdrant"],
        ports: &[6333],
        systemd_units: &["qdrant.service"],
    },
    ServiceDefinition {
        id: "solr",
        name: "Apache Solr",
        category: "Search",
        binaries: &["solr"],
        // Solr runs on Java; matching the generic JVM process would be misleading.
        processes: &[],
        ports: &[8983],
        systemd_units: &["solr.service"],
    },
    ServiceDefinition {
        id: "mailpit",
        name: "Mailpit",
        category: "Email",
        binaries: &["mailpit"],
        processes: &["mailpit"],
        ports: &[1025, 8025],
        systemd_units: &["mailpit.service"],
    },
    ServiceDefinition {
        id: "garage",
        name: "Garage S3",
        category: "Object storage",
        binaries: &["garage"],
        processes: &["garage"],
        ports: &[3900],
        systemd_units: &["garage.service"],
    },
    ServiceDefinition {
        id: "minio",
        name: "MinIO",
        category: "Object storage",
        binaries: &["minio"],
        processes: &["minio"],
        ports: &[9000],
        systemd_units: &["minio.service", "minio-server.service"],
    },
    ServiceDefinition {
        id: "mailhog",
        name: "MailHog",
        category: "Email",
        binaries: &["MailHog"],
        processes: &["MailHog"],
        ports: &[1025, 8025],
        systemd_units: &["mailhog.service"],
    },
    ServiceDefinition {
        id: "localstack",
        name: "LocalStack",
        category: "Local cloud",
        binaries: &["localstack"],
        processes: &["localstack"],
        ports: &[4566],
        systemd_units: &["localstack.service"],
    },
    ServiceDefinition {
        id: "influxdb",
        name: "InfluxDB",
        category: "Observability",
        binaries: &["influxd"],
        processes: &["influxd"],
        ports: &[8086],
        systemd_units: &["influxdb.service"],
    },
    ServiceDefinition {
        id: "prometheus",
        name: "Prometheus",
        category: "Observability",
        binaries: &["prometheus"],
        processes: &["prometheus"],
        ports: &[9090],
        systemd_units: &["prometheus.service"],
    },
    ServiceDefinition {
        id: "grafana",
        name: "Grafana",
        category: "Observability",
        binaries: &["grafana", "grafana-server"],
        processes: &["grafana", "grafana-server"],
        ports: &[3000],
        systemd_units: &["grafana-server.service"],
    },
    ServiceDefinition {
        id: "caddy",
        name: "Caddy",
        category: "Web server",
        binaries: &["caddy"],
        processes: &["caddy"],
        ports: &[80, 443],
        systemd_units: &["caddy.service"],
    },
    ServiceDefinition {
        id: "nginx",
        name: "Nginx",
        category: "Web server",
        binaries: &["nginx"],
        processes: &["nginx"],
        ports: &[80, 443],
        systemd_units: &["nginx.service"],
    },
    ServiceDefinition {
        id: "apache",
        name: "Apache HTTP Server",
        category: "Web server",
        binaries: &["apache2", "httpd"],
        processes: &["apache2", "httpd"],
        ports: &[80, 443],
        systemd_units: &["apache2.service", "httpd.service"],
    },
    ServiceDefinition {
        id: "traefik",
        name: "Traefik",
        category: "Web server",
        binaries: &["traefik"],
        processes: &["traefik"],
        ports: &[80, 443],
        systemd_units: &["traefik.service"],
    },
    ServiceDefinition {
        id: "haproxy",
        name: "HAProxy",
        category: "Web server",
        binaries: &["haproxy"],
        processes: &["haproxy"],
        ports: &[80, 443],
        systemd_units: &["haproxy.service"],
    },
    ServiceDefinition {
        id: "docker",
        name: "Docker Engine",
        category: "Container runtime",
        binaries: &["dockerd"],
        processes: &["dockerd"],
        ports: &[],
        systemd_units: &["docker.service"],
    },
    ServiceDefinition {
        id: "containerd",
        name: "containerd",
        category: "Container runtime",
        binaries: &["containerd"],
        processes: &["containerd"],
        ports: &[],
        systemd_units: &["containerd.service"],
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
    let systemd_units = discover_systemd_units().await;
    let discovered = Arc::new(discovered);
    let systemd_units = Arc::new(systemd_units);
    let definitions = SERVICES.to_vec();
    stream::iter(definitions.into_iter().map(|definition| {
        let discovered = Arc::clone(&discovered);
        let systemd_units = Arc::clone(&systemd_units);
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
            let systemd = systemd_units.get(definition.id);
            let state = if process_count > 0 {
                if reachable_port.is_some() || definition.ports.is_empty() {
                    ServiceState::Running
                } else {
                    ServiceState::Unhealthy
                }
            } else if reachable_port.is_some() {
                // A listener may be an externally managed service even when its server binary is not in PATH.
                ServiceState::EndpointReachable
            } else if systemd.is_some_and(|unit| unit.active_state == "active") {
                // This covers services such as Kafka whose actual process is a generic JVM.
                ServiceState::Running
            } else if executable_path.is_empty() && systemd.is_none() {
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
                systemd_unit: systemd.map(|unit| unit.name.clone()),
                systemd_active_state: systemd.map(|unit| unit.active_state.clone()),
                systemd_sub_state: systemd.map(|unit| unit.sub_state.clone()),
            }
        }
    }))
    .buffer_unordered(8)
    .collect()
    .await
}

#[derive(Clone)]
struct SystemdUnitStatus {
    name: String,
    active_state: String,
    sub_state: String,
}

async fn discover_systemd_units() -> HashMap<&'static str, SystemdUnitStatus> {
    #[cfg(target_os = "linux")]
    {
        let Ok(units) = list_systemd_service_units().await else {
            return HashMap::new();
        };
        let definitions = SERVICES.to_vec();
        stream::iter(definitions.into_iter().map(|definition| {
            let units = units.clone();
            async move {
                for candidate in definition.systemd_units {
                    if !units.contains(*candidate) {
                        continue;
                    }
                    if let Ok(Some(status)) = read_systemd_unit_status(candidate).await {
                        return Some((definition.id, status));
                    }
                }
                None
            }
        }))
        .buffer_unordered(8)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .flatten()
        .collect()
    }
    #[cfg(not(target_os = "linux"))]
    {
        HashMap::new()
    }
}

#[cfg(target_os = "linux")]
async fn list_systemd_service_units() -> Result<HashSet<String>> {
    let output = tokio::time::timeout(
        Duration::from_secs(3),
        Command::new("systemctl")
            .args([
                "--no-pager",
                "list-unit-files",
                "--type=service",
                "--no-legend",
            ])
            .output(),
    )
    .await
    .map_err(|_| anyhow!("timed out while querying systemd service units"))?
    .context("running systemctl to discover host service units")?;
    if !output.status.success() {
        return Err(anyhow!(
            "systemctl could not list service units: {}",
            command_output_detail(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|unit| unit.ends_with(".service"))
        .map(str::to_string)
        .collect())
}

#[cfg(target_os = "linux")]
async fn read_systemd_unit_status(unit: &str) -> Result<Option<SystemdUnitStatus>> {
    let output = tokio::time::timeout(
        Duration::from_secs(3),
        Command::new("systemctl")
            .args([
                "--no-pager",
                "show",
                "--property=LoadState",
                "--property=ActiveState",
                "--property=SubState",
                unit,
            ])
            .output(),
    )
    .await
    .map_err(|_| anyhow!("timed out while reading systemd unit {unit}"))?
    .with_context(|| format!("reading systemd unit {unit}"))?;
    if !output.status.success() {
        return Err(anyhow!(
            "systemctl could not read {unit}: {}",
            command_output_detail(&output.stderr)
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let properties = stdout
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect::<HashMap<_, _>>();
    if properties.get("LoadState").map(String::as_str) != Some("loaded") {
        return Ok(None);
    }
    let Some(active_state) = properties.get("ActiveState") else {
        return Ok(None);
    };
    let sub_state = properties
        .get("SubState")
        .map(String::as_str)
        .unwrap_or("unknown");
    Ok(Some(SystemdUnitStatus {
        name: unit.to_string(),
        active_state: active_state.clone(),
        sub_state: sub_state.to_string(),
    }))
}

#[cfg(target_os = "linux")]
pub async fn control_host_service(service_id: &str, action: HostServiceAction) -> Result<()> {
    let definition = SERVICES
        .iter()
        .find(|definition| definition.id == service_id)
        .ok_or_else(|| anyhow!("unknown host service"))?;
    let units = list_systemd_service_units().await?;
    let unit = definition
        .systemd_units
        .iter()
        .find(|candidate| units.contains(**candidate))
        .copied()
        .ok_or_else(|| {
            anyhow!(
                "no supported systemd service unit was found for {}",
                definition.name
            )
        })?;

    let mut command = Command::new("systemctl");
    command
        .args(["--no-pager", action.systemctl_verb(), unit])
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(90), command.output())
        .await
        .map_err(|_| {
            anyhow!(
                "timed out while asking systemd to {} {unit}",
                action.systemctl_verb()
            )
        })?
        .with_context(|| format!("asking systemd to {} {unit}", action.systemctl_verb()))?;
    if !output.status.success() {
        return Err(anyhow!(
            "systemd could not {} {unit}: {}",
            action.systemctl_verb(),
            command_output_detail(&output.stderr)
        ));
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub async fn control_host_service(_service_id: &str, _action: HostServiceAction) -> Result<()> {
    Err(anyhow!("host service controls require Linux with systemd"))
}

fn command_output_detail(bytes: &[u8]) -> String {
    let detail = String::from_utf8_lossy(bytes).trim().to_string();
    if detail.is_empty() {
        "no additional details were returned".into()
    } else {
        detail.chars().take(600).collect()
    }
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
