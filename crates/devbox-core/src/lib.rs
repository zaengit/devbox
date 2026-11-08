mod discovery;
mod extensions;
mod model;
mod php_extensions;
mod platform;
mod process;
mod project_detection;
mod proxy;
mod services;
mod storage;
mod workspaces;

pub use extensions::{
    EXTENSION_API_VERSION, SUPPORTED_CAPABILITIES as SUPPORTED_EXTENSION_CAPABILITIES,
};

pub use discovery::{
    ProbeResult, extract_version, parse_arguments, scan_path, scan_path_with_directories,
    test_binary, validate_executable,
};
pub use model::{
    BinaryCategory, BinaryRecord, EnvironmentVariable, ExtensionRecord, LocalDomain,
    ManagedServiceProfile, ProcessSnapshot, ProcessStatus, Project, Workspace,
};
pub use php_extensions::PhpExtensionReport;
pub use process::ProcessManager;
pub use project_detection::detect_frameworks;
pub use services::{
    ManagedServiceSnapshot, ServiceManager, ServiceSnapshot, ServiceState, scan_services,
};
pub use storage::Storage;

use anyhow::{Context, Result, anyhow};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub struct Core {
    pub storage: Storage,
    pub processes: ProcessManager,
    pub services: ServiceManager,
    workspace_service_consumers: Arc<std::sync::Mutex<HashMap<String, HashSet<String>>>>,
    workspace_started_services: Arc<std::sync::Mutex<HashSet<String>>>,
}

impl Core {
    pub fn open_default() -> Result<Self> {
        let storage = Storage::open_default()?;
        storage.reconcile_processes()?;
        Ok(Self {
            storage,
            processes: ProcessManager::default(),
            services: ServiceManager::default(),
            workspace_service_consumers: Arc::default(),
            workspace_started_services: Arc::default(),
        })
    }

    pub fn open(path: impl Into<std::path::PathBuf>) -> Result<Self> {
        let storage = Storage::open(path)?;
        storage.reconcile_processes()?;
        Ok(Self {
            storage,
            processes: ProcessManager::default(),
            services: ServiceManager::default(),
            workspace_service_consumers: Arc::default(),
            workspace_started_services: Arc::default(),
        })
    }

    pub async fn load_binaries(&self) -> Result<Vec<BinaryRecord>> {
        let storage = self.storage.clone();
        tokio::task::spawn_blocking(move || storage.list_binaries()).await?
    }

    pub async fn load_projects(&self) -> Result<Vec<Project>> {
        let storage = self.storage.clone();
        tokio::task::spawn_blocking(move || storage.list_projects()).await?
    }

    pub async fn load_dark_mode(&self) -> Result<bool> {
        let storage = self.storage.clone();
        let value =
            tokio::task::spawn_blocking(move || storage.setting("appearance.dark_mode")).await??;
        Ok(value.as_deref() == Some("true"))
    }

    pub async fn save_dark_mode(&self, enabled: bool) -> Result<()> {
        let storage = self.storage.clone();
        tokio::task::spawn_blocking(move || {
            storage.set_setting(
                "appearance.dark_mode",
                if enabled { "true" } else { "false" },
            )
        })
        .await??;
        Ok(())
    }

    pub async fn load_discovery_settings(&self) -> Result<(Vec<String>, bool)> {
        let storage = self.storage.clone();
        tokio::task::spawn_blocking(move || {
            let directories = storage
                .setting("discovery.search_directories")?
                .map(|value| serde_json::from_str(&value))
                .transpose()?
                .unwrap_or_default();
            let automatic_rescan = storage
                .setting("discovery.automatic_rescan")?
                .map(|value| value == "true")
                .unwrap_or(true);
            Ok::<_, anyhow::Error>((directories, automatic_rescan))
        })
        .await?
    }

    pub async fn save_discovery_settings(
        &self,
        directories: Vec<String>,
        automatic_rescan: bool,
    ) -> Result<(Vec<String>, bool)> {
        const MAX_DIRECTORIES: usize = 64;
        if directories.len() > MAX_DIRECTORIES {
            return Err(anyhow!(
                "at most {MAX_DIRECTORIES} binary search directories can be configured"
            ));
        }
        let mut normalized = Vec::new();
        for directory in directories {
            let path = PathBuf::from(&directory)
                .canonicalize()
                .map_err(|error| anyhow!("cannot resolve search directory {directory}: {error}"))?;
            if !path.is_dir() {
                return Err(anyhow!("search path {} is not a directory", path.display()));
            }
            let path = path.display().to_string();
            if !normalized.contains(&path) {
                normalized.push(path);
            }
        }
        let storage = self.storage.clone();
        let persisted = normalized.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            storage.set_setting(
                "discovery.search_directories",
                &serde_json::to_string(&persisted)?,
            )?;
            storage.set_setting(
                "discovery.automatic_rescan",
                if automatic_rescan { "true" } else { "false" },
            )?;
            Ok(())
        })
        .await??;
        Ok((normalized, automatic_rescan))
    }

    pub async fn initial_binaries(&self) -> Result<Vec<BinaryRecord>> {
        let (_, automatic_rescan) = self.load_discovery_settings().await?;
        if automatic_rescan {
            self.rescan_binaries().await
        } else {
            self.load_binaries().await
        }
    }

    pub async fn rescan_binaries(&self) -> Result<Vec<BinaryRecord>> {
        let (directories, _) = self.load_discovery_settings().await?;
        let discovered =
            scan_path_with_directories(directories.into_iter().map(PathBuf::from).collect()).await;
        let storage = self.storage.clone();
        let discovered = tokio::task::spawn_blocking(move || -> Result<Vec<BinaryRecord>> {
            for binary in discovered {
                storage.save_discovered_binary(&binary)?;
            }
            storage.list_binaries()
        })
        .await??;
        Ok(discovered)
    }

    pub async fn save_custom_binary(
        &self,
        display_name: String,
        category: BinaryCategory,
        path: String,
        arguments: Vec<String>,
        version_regex: Option<String>,
    ) -> Result<Vec<BinaryRecord>> {
        let path = Path::new(&path);
        let resolved = validate_executable(path)?;
        let resolved_text = resolved.display().to_string();
        let mut binary = BinaryRecord {
            id: Uuid::new_v4().to_string(),
            display_name,
            category,
            executable_path: resolved_text.clone(),
            resolved_path: Some(resolved_text),
            version_arguments: arguments,
            version_parser: if version_regex.is_some() {
                "regex"
            } else {
                "auto"
            }
            .to_string(),
            version_regex,
            detected_version: None,
            probe_error: None,
            manual_version: None,
            discovery_source: "Custom registration".to_string(),
            trust_status: "Not tested".to_string(),
            enabled: true,
            last_verified_at: None,
        };
        let probe = test_binary(
            Path::new(&binary.executable_path),
            &binary.version_arguments,
            binary.version_regex.as_deref(),
        )
        .await;
        let Some(version) = probe.version else {
            return Err(anyhow!(
                probe.error.unwrap_or_else(|| "version check failed".into())
            ));
        };
        binary.detected_version = Some(version);
        binary.trust_status = "Available".to_string();
        binary.last_verified_at = Some(crate::model::now_unix());
        let storage = self.storage.clone();
        let binaries = tokio::task::spawn_blocking(move || -> Result<Vec<BinaryRecord>> {
            storage.save_binary(&binary)?;
            storage.list_binaries()
        })
        .await??;
        Ok(binaries)
    }

    pub async fn test_path(
        &self,
        path: String,
        arguments: Vec<String>,
        version_regex: Option<String>,
    ) -> ProbeResult {
        test_binary(Path::new(&path), &arguments, version_regex.as_deref()).await
    }

    pub async fn inspect_php_extensions(&self, binary_id: String) -> Result<PhpExtensionReport> {
        let storage = self.storage.clone();
        let binary = tokio::task::spawn_blocking(move || -> Result<_> {
            storage
                .list_binaries()?
                .into_iter()
                .find(|binary| binary.id == binary_id)
                .ok_or_else(|| anyhow!("PHP binary registration no longer exists"))
        })
        .await??;
        if !binary.enabled {
            return Err(anyhow!("the selected PHP registration is disabled"));
        }
        if !matches!(&binary.category, BinaryCategory::Runtime)
            || !binary.display_name.to_ascii_lowercase().contains("php")
        {
            return Err(anyhow!(
                "select a registered PHP runtime to inspect extensions"
            ));
        }
        let executable = binary
            .resolved_path
            .as_deref()
            .unwrap_or(&binary.executable_path);
        let resolved = validate_executable(Path::new(executable))
            .map_err(|error| anyhow!("selected PHP runtime is unavailable: {error}"))?;
        php_extensions::inspect_php_extensions(&resolved).await
    }

    pub async fn remove_binary(&self, id: String) -> Result<Vec<BinaryRecord>> {
        let storage = self.storage.clone();
        let binaries = tokio::task::spawn_blocking(move || -> Result<Vec<BinaryRecord>> {
            storage.remove_binary(&id)?;
            storage.list_binaries()
        })
        .await??;
        Ok(binaries)
    }

    pub async fn add_project(
        &self,
        name: String,
        path: String,
        runtime_binary_id: Option<String>,
        startup_arguments: Vec<String>,
    ) -> Result<Vec<Project>> {
        self.add_project_with_environment(
            name,
            path,
            runtime_binary_id,
            startup_arguments,
            Vec::new(),
        )
        .await
    }

    pub async fn add_project_with_environment(
        &self,
        name: String,
        path: String,
        runtime_binary_id: Option<String>,
        startup_arguments: Vec<String>,
        environment_variables: Vec<EnvironmentVariable>,
    ) -> Result<Vec<Project>> {
        let project_path = Path::new(&path)
            .canonicalize()
            .with_context(|| format!("resolving project directory {path}"))?;
        if !project_path.is_dir() {
            return Err(anyhow!("project path must be a directory"));
        }
        let environment_variables = validate_environment_variables(environment_variables)?;
        let project = Project {
            id: Uuid::new_v4().to_string(),
            name,
            path: project_path.display().to_string(),
            runtime_binary_id,
            startup_arguments,
            environment_variables,
            detected_frameworks: detect_frameworks(&project_path),
            created_at: crate::model::now_unix(),
        };
        let storage = self.storage.clone();
        let projects = tokio::task::spawn_blocking(move || -> Result<Vec<Project>> {
            storage.save_project(&project)?;
            storage.list_projects()
        })
        .await??;
        Ok(projects)
    }

    pub async fn assign_runtime(
        &self,
        project_id: String,
        binary_id: Option<String>,
    ) -> Result<Vec<Project>> {
        let storage = self.storage.clone();
        let projects = tokio::task::spawn_blocking(move || -> Result<Vec<Project>> {
            storage.set_project_runtime(&project_id, binary_id.as_deref())?;
            storage.list_projects()
        })
        .await??;
        Ok(projects)
    }

    pub async fn update_project_configuration(
        &self,
        project_id: String,
        runtime_binary_id: Option<String>,
        startup_arguments: Vec<String>,
        environment_variables: Vec<EnvironmentVariable>,
    ) -> Result<Vec<Project>> {
        if startup_arguments.len() > 128
            || startup_arguments
                .iter()
                .any(|argument| argument.contains('\0'))
        {
            return Err(anyhow!("project startup arguments contain invalid values"));
        }
        let environment_variables = validate_environment_variables(environment_variables)?;
        let storage = self.storage.clone();
        let projects = tokio::task::spawn_blocking(move || -> Result<Vec<Project>> {
            storage.update_project_configuration(
                &project_id,
                runtime_binary_id.as_deref(),
                &startup_arguments,
                &environment_variables,
            )?;
            storage.list_projects()
        })
        .await??;
        Ok(projects)
    }

    pub async fn start_project(&self, project_id: String) -> Result<ProcessSnapshot> {
        let (project, resolved) = self.project_runtime(project_id).await?;
        self.processes
            .start(self.storage.clone(), project, &resolved)
            .await
    }

    pub async fn restart_project(&self, project_id: String) -> Result<ProcessSnapshot> {
        let (project, resolved) = self.project_runtime(project_id).await?;
        self.processes
            .restart(self.storage.clone(), project, &resolved)
            .await
    }

    pub async fn stop_project(&self, project_id: String) -> Result<()> {
        self.processes.stop(self.storage.clone(), &project_id).await
    }

    pub fn process_snapshots(&self) -> Vec<ProcessSnapshot> {
        self.processes.snapshots()
    }

    pub fn managed_service_snapshots(&self) -> Vec<ManagedServiceSnapshot> {
        self.services.snapshots()
    }

    pub async fn load_managed_service_profiles(&self) -> Result<Vec<ManagedServiceProfile>> {
        let storage = self.storage.clone();
        tokio::task::spawn_blocking(move || storage.list_managed_services()).await?
    }

    pub async fn save_managed_service_profile(
        &self,
        mut profile: ManagedServiceProfile,
    ) -> Result<Vec<ManagedServiceProfile>> {
        if profile.name.trim().is_empty() || profile.name.len() > 100 {
            return Err(anyhow!(
                "service name is required and must be under 100 characters"
            ));
        }
        if profile.kind.trim().is_empty() || profile.kind.len() > 60 {
            return Err(anyhow!("service kind is required"));
        }
        if profile.ports.len() > 16 || profile.ports.iter().any(|port| *port < 1024) {
            return Err(anyhow!(
                "configure up to 16 unprivileged service ports (1024–65535)"
            ));
        }
        let mut ports = profile.ports.clone();
        ports.sort_unstable();
        ports.dedup();
        if ports.len() != profile.ports.len() {
            return Err(anyhow!("service ports must be unique"));
        }
        if profile.arguments.len() > 128 || profile.arguments.iter().any(|arg| arg.contains('\0')) {
            return Err(anyhow!("service arguments contain invalid values"));
        }
        let executable = validate_executable(Path::new(&profile.executable_path))?;
        profile.executable_path = executable.display().to_string();
        if let Some(directory) = &profile.data_directory {
            let path = Path::new(directory)
                .canonicalize()
                .with_context(|| format!("resolving service data directory {directory}"))?;
            if !path.is_dir() {
                return Err(anyhow!("service data path must be a directory"));
            }
            #[cfg(target_os = "linux")]
            {
                use std::os::unix::fs::MetadataExt;
                if std::fs::metadata(&path)?.uid() != unsafe { libc::geteuid() } {
                    return Err(anyhow!(
                        "service data directory must be owned by the current user"
                    ));
                }
            }
            profile.data_directory = Some(path.display().to_string());
        }
        if profile.id.is_empty() {
            profile.id = Uuid::new_v4().to_string();
        }
        if profile.created_at == 0 {
            profile.created_at = crate::model::now_unix();
        }
        let storage = self.storage.clone();
        tokio::task::spawn_blocking(move || {
            storage.save_managed_service(&profile)?;
            storage.list_managed_services()
        })
        .await?
    }

    pub async fn remove_managed_service_profile(
        &self,
        id: String,
    ) -> Result<Vec<ManagedServiceProfile>> {
        if self.services.snapshots().iter().any(|snapshot| {
            snapshot.service_id == id
                && (snapshot.status == "running" || snapshot.status == "stopping")
        }) {
            return Err(anyhow!(
                "stop this Devbox-managed service before removing its profile"
            ));
        }
        let storage = self.storage.clone();
        tokio::task::spawn_blocking(move || {
            storage.remove_managed_service(&id)?;
            storage.list_managed_services()
        })
        .await?
    }

    pub async fn start_managed_service(&self, id: String) -> Result<ManagedServiceSnapshot> {
        let storage = self.storage.clone();
        let profile = tokio::task::spawn_blocking(move || {
            storage
                .list_managed_services()?
                .into_iter()
                .find(|profile| profile.id == id)
                .ok_or_else(|| anyhow!("managed service profile no longer exists"))
        })
        .await??;
        self.services.start(&profile).await
    }

    pub async fn stop_managed_service(&self, id: String) -> Result<()> {
        self.services.stop(&id).await
    }

    pub async fn restart_managed_service(&self, id: String) -> Result<ManagedServiceSnapshot> {
        let storage = self.storage.clone();
        let profile = tokio::task::spawn_blocking(move || {
            storage
                .list_managed_services()?
                .into_iter()
                .find(|profile| profile.id == id)
                .ok_or_else(|| anyhow!("managed service profile no longer exists"))
        })
        .await??;
        self.services.stop(&profile.id).await?;
        for _ in 0..30 {
            if self
                .services
                .snapshots()
                .iter()
                .find(|state| state.service_id == profile.id)
                .is_some_and(|state| state.status != "running" && state.status != "stopping")
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        self.services.start(&profile).await
    }

    pub async fn create_workspace(&self, name: String) -> Result<Vec<Workspace>> {
        let name = name.trim().to_string();
        if name.is_empty() || name.len() > 100 {
            return Err(anyhow!(
                "workspace name is required and must be under 100 characters"
            ));
        }
        let id = Uuid::new_v4().to_string();
        let storage = self.storage.clone();
        tokio::task::spawn_blocking(move || {
            storage.create_workspace(&id, &name, crate::model::now_unix())?;
            storage.list_workspaces()
        })
        .await?
    }

    pub async fn add_project_to_workspace(
        &self,
        workspace_id: String,
        project_id: String,
    ) -> Result<Vec<Workspace>> {
        let storage = self.storage.clone();
        tokio::task::spawn_blocking(move || {
            storage.add_workspace_project(&workspace_id, &project_id)?;
            storage.list_workspaces()
        })
        .await?
    }

    pub async fn set_workspace_dependency(
        &self,
        workspace_id: String,
        project_id: String,
        prerequisite_id: String,
        depends: bool,
    ) -> Result<Vec<Workspace>> {
        let storage = self.storage.clone();
        let workspaces = tokio::task::spawn_blocking(move || -> Result<Vec<Workspace>> {
            storage.set_project_dependency(
                &workspace_id,
                &project_id,
                &prerequisite_id,
                depends,
            )?;
            let workspaces = storage.list_workspaces()?;
            let workspace = workspaces
                .iter()
                .find(|workspace| workspace.id == workspace_id)
                .ok_or_else(|| anyhow!("workspace no longer exists"))?;
            if let Err(error) = workspaces::startup_order(workspace) {
                let _ = storage.set_project_dependency(
                    &workspace_id,
                    &project_id,
                    &prerequisite_id,
                    !depends,
                );
                return Err(error);
            }
            Ok(workspaces)
        })
        .await??;
        Ok(workspaces)
    }

    pub async fn attach_service_to_workspace(
        &self,
        workspace_id: String,
        service_id: String,
    ) -> Result<Vec<Workspace>> {
        let storage = self.storage.clone();
        tokio::task::spawn_blocking(move || {
            storage.add_workspace_service(&workspace_id, &service_id)?;
            storage.list_workspaces()
        })
        .await?
    }

    pub async fn start_workspace(&self, workspace_id: String) -> Result<Vec<ProcessSnapshot>> {
        let storage = self.storage.clone();
        let workspace = tokio::task::spawn_blocking(move || {
            storage
                .list_workspaces()?
                .into_iter()
                .find(|workspace| workspace.id == workspace_id)
                .ok_or_else(|| anyhow!("workspace no longer exists"))
        })
        .await??;
        let order = workspaces::startup_order(&workspace)?;
        let service_ids = {
            let storage = self.storage.clone();
            let workspace_id_for_services = workspace.id.clone();
            tokio::task::spawn_blocking(move || {
                storage.workspace_services(&workspace_id_for_services)
            })
            .await??
        };
        let profiles = self.load_managed_service_profiles().await?;
        let running = self
            .services
            .snapshots()
            .into_iter()
            .filter(|state| state.status == "running")
            .map(|state| state.service_id)
            .collect::<HashSet<_>>();
        for service_id in &service_ids {
            let already_running = running.contains(service_id);
            let Some(profile) = profiles.iter().find(|profile| &profile.id == service_id) else {
                self.release_workspace_services(&workspace.id).await;
                return Err(anyhow!(
                    "workspace references a missing managed service profile"
                ));
            };
            if !already_running {
                if let Err(error) = self.services.start(profile).await {
                    self.release_workspace_services(&workspace.id).await;
                    return Err(anyhow!(
                        "workspace service startup failed for {}: {error}",
                        profile.name
                    ));
                }
                if let Ok(mut started) = self.workspace_started_services.lock() {
                    started.insert(service_id.clone());
                }
            }
            if let Ok(mut consumers) = self.workspace_service_consumers.lock() {
                consumers
                    .entry(service_id.clone())
                    .or_default()
                    .insert(workspace.id.clone());
            }
            if let Err(error) = self
                .services
                .wait_until_ready(service_id, &profile.ports)
                .await
            {
                self.release_workspace_services(&workspace.id).await;
                return Err(anyhow!(
                    "workspace service {} failed its readiness check: {error}",
                    profile.name
                ));
            }
        }
        let mut started_projects = Vec::new();
        for project_id in order {
            if self.processes.snapshots().iter().any(|process| {
                process.project_id == project_id && process.status == ProcessStatus::Running
            }) {
                continue;
            }
            match self.start_project(project_id.clone()).await {
                Ok(_) => started_projects.push(project_id),
                Err(error) => {
                    for id in started_projects.iter().rev() {
                        let _ = self.stop_project(id.clone()).await;
                    }
                    self.release_workspace_services(&workspace.id).await;
                    return Err(anyhow!(
                        "workspace startup stopped after a project failed; newly started projects were stopped for recovery: {error}"
                    ));
                }
            }
        }
        Ok(self
            .processes
            .snapshots()
            .into_iter()
            .filter(|process| workspace.project_ids.contains(&process.project_id))
            .collect())
    }

    pub async fn stop_workspace(&self, workspace_id: String) -> Result<()> {
        let storage = self.storage.clone();
        let workspace = tokio::task::spawn_blocking(move || {
            storage
                .list_workspaces()?
                .into_iter()
                .find(|workspace| workspace.id == workspace_id)
                .ok_or_else(|| anyhow!("workspace no longer exists"))
        })
        .await??;
        let order = workspaces::startup_order(&workspace)?;
        let mut failures = Vec::new();
        for project_id in order.iter().rev() {
            if self.processes.snapshots().iter().any(|process| {
                process.project_id == *project_id && process.status == ProcessStatus::Running
            }) && let Err(error) = self.stop_project(project_id.clone()).await
            {
                failures.push(error.to_string());
            }
        }
        self.release_workspace_services(&workspace.id).await;
        if failures.is_empty() {
            Ok(())
        } else {
            Err(anyhow!(
                "some workspace processes did not stop: {}",
                failures.join("; ")
            ))
        }
    }

    async fn release_workspace_services(&self, workspace_id: &str) {
        let service_ids = if let Ok(mut consumers) = self.workspace_service_consumers.lock() {
            let mut releasable = Vec::new();
            for (service_id, workspaces) in consumers.iter_mut() {
                workspaces.remove(workspace_id);
                if workspaces.is_empty() {
                    releasable.push(service_id.clone());
                }
            }
            releasable
        } else {
            Vec::new()
        };
        for service_id in service_ids {
            let should_stop = self
                .workspace_started_services
                .lock()
                .map(|mut started| started.remove(&service_id))
                .unwrap_or(false);
            if should_stop {
                let _ = self.services.stop(&service_id).await;
            }
        }
    }

    pub async fn register_extension(&self, manifest_path: String) -> Result<Vec<ExtensionRecord>> {
        let extension = tokio::task::spawn_blocking(move || {
            extensions::read_manifest(Path::new(&manifest_path))
        })
        .await??;
        let storage = self.storage.clone();
        tokio::task::spawn_blocking(move || {
            storage.save_extension(&extension)?;
            storage.list_extensions()
        })
        .await?
    }

    pub async fn load_extensions(&self) -> Result<Vec<ExtensionRecord>> {
        let storage = self.storage.clone();
        tokio::task::spawn_blocking(move || storage.list_extensions()).await?
    }

    pub async fn approve_extension_and_enable(&self, id: String) -> Result<Vec<ExtensionRecord>> {
        let storage = self.storage.clone();
        let extension = tokio::task::spawn_blocking(move || {
            storage
                .list_extensions()?
                .into_iter()
                .find(|extension| extension.id == id)
                .ok_or_else(|| anyhow!("extension registration no longer exists"))
        })
        .await??;
        self.set_extension_permissions(extension.id, extension.capabilities, true)
            .await
    }

    pub async fn set_extension_permissions(
        &self,
        id: String,
        approved: Vec<String>,
        enable: bool,
    ) -> Result<Vec<ExtensionRecord>> {
        let storage = self.storage.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<ExtensionRecord>> {
            let mut extension = storage
                .list_extensions()?
                .into_iter()
                .find(|extension| extension.id == id)
                .ok_or_else(|| anyhow!("extension registration no longer exists"))?;
            let mut approved = approved;
            approved.sort();
            approved.dedup();
            if let Some(unknown) = approved.iter().find(|capability| !extension.capabilities.contains(capability)) {
                return Err(anyhow!("cannot approve undeclared extension capability {unknown}"));
            }
            let all_approved = extension.capabilities.iter().all(|capability| approved.contains(capability));
            extension.approved_capabilities = approved;
            extension.enabled = enable && all_approved;
            storage.save_extension(&extension)?;
            storage.set_extension_enabled(&extension.id, extension.enabled)?;
            if enable && !all_approved {
                return Err(anyhow!("extension remains disabled until every required capability is explicitly approved"));
            }
            storage.list_extensions()
        })
        .await?
    }

    pub async fn disable_extension(&self, id: String) -> Result<Vec<ExtensionRecord>> {
        let storage = self.storage.clone();
        tokio::task::spawn_blocking(move || {
            storage.set_extension_enabled(&id, false)?;
            storage.list_extensions()
        })
        .await?
    }

    pub async fn remove_extension(&self, id: String) -> Result<Vec<ExtensionRecord>> {
        let storage = self.storage.clone();
        tokio::task::spawn_blocking(move || {
            storage.remove_extension(&id)?;
            storage.list_extensions()
        })
        .await?
    }

    pub async fn save_local_domain(&self, mut domain: LocalDomain) -> Result<Vec<LocalDomain>> {
        let storage = self.storage.clone();
        let existing = tokio::task::spawn_blocking(move || storage.list_local_domains()).await??;
        if domain.id.is_empty() {
            domain.id = Uuid::new_v4().to_string();
        }
        let mut proposed = existing
            .into_iter()
            .filter(|route| route.id != domain.id)
            .collect::<Vec<_>>();
        proposed.push(domain.clone());
        proxy::validate_domains(&proposed)?;
        let storage = self.storage.clone();
        tokio::task::spawn_blocking(move || {
            storage.save_local_domain(&domain)?;
            storage.list_local_domains()
        })
        .await?
    }

    pub async fn remove_local_domain(&self, id: String) -> Result<Vec<LocalDomain>> {
        let storage = self.storage.clone();
        tokio::task::spawn_blocking(move || {
            storage.remove_local_domain(&id)?;
            storage.list_local_domains()
        })
        .await?
    }

    pub async fn render_proxy_config(&self) -> Result<String> {
        let storage = self.storage.clone();
        let domains = tokio::task::spawn_blocking(move || storage.list_local_domains()).await??;
        proxy::render_caddyfile(&domains)
    }

    pub async fn validate_caddy_config(
        &self,
        executable: String,
        config: String,
    ) -> Result<String> {
        proxy::validate_caddyfile(Path::new(&executable), Path::new(&config)).await
    }

    pub async fn apply_proxy_config(&self, executable: String, target: String) -> Result<String> {
        let config = self.render_proxy_config().await?;
        proxy::install_caddyfile(Path::new(&executable), Path::new(&target), config).await
    }

    pub async fn restore_proxy_config(&self, target: String) -> Result<String> {
        proxy::restore_caddyfile(Path::new(&target)).await
    }

    async fn project_runtime(&self, project_id: String) -> Result<(Project, std::path::PathBuf)> {
        let storage = self.storage.clone();
        let (project, binary) = tokio::task::spawn_blocking(move || -> Result<_> {
            let project = storage.project(&project_id)?;
            let binary_id = project
                .runtime_binary_id
                .as_deref()
                .ok_or_else(|| anyhow!("select a runtime for this project first"))?;
            let binary = storage
                .list_binaries()?
                .into_iter()
                .find(|binary| binary.id == binary_id)
                .ok_or_else(|| anyhow!("the selected runtime registration is missing"))?;
            Ok((project, binary))
        })
        .await??;
        if !binary.enabled {
            return Err(anyhow!("the selected binary registration is disabled"));
        }
        let executable = binary
            .resolved_path
            .as_deref()
            .unwrap_or(&binary.executable_path);
        let resolved = validate_executable(Path::new(executable)).map_err(|error| {
            anyhow!(
                "selected runtime {} is unavailable: {error}",
                binary.display_name
            )
        })?;
        Ok((project, resolved))
    }
}

fn validate_environment_variables(
    variables: Vec<EnvironmentVariable>,
) -> Result<Vec<EnvironmentVariable>> {
    let mut keys = std::collections::HashSet::new();
    let mut total_bytes = 0usize;
    for variable in &variables {
        let mut characters = variable.key.chars();
        let valid_first = characters
            .next()
            .is_some_and(|character| character == '_' || character.is_ascii_alphabetic());
        if !valid_first
            || !characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
        {
            return Err(anyhow!(
                "environment variable names must start with a letter or underscore and contain only letters, digits, and underscores"
            ));
        }
        if variable
            .value
            .chars()
            .any(|character| matches!(character, '\0' | '\n' | '\r'))
        {
            return Err(anyhow!(
                "environment variable values cannot contain NUL or newline characters"
            ));
        }
        if !keys.insert(&variable.key) {
            return Err(anyhow!(
                "environment variable {} is listed more than once",
                variable.key
            ));
        }
        total_bytes = total_bytes.saturating_add(variable.key.len() + variable.value.len());
    }
    if total_bytes > 256 * 1024 {
        return Err(anyhow!("project environment exceeds the 256 KiB limit"));
    }
    Ok(variables)
}

pub type SharedCore = Arc<Core>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::time::Duration;
    use tempfile::TempDir;

    fn fixture_named(temp: &TempDir, name: &str, body: &str) -> PathBuf {
        let path = temp.path().join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }

    fn fixture(temp: &TempDir, body: &str) -> PathBuf {
        fixture_named(temp, "fixture-runtime", body)
    }

    fn storage_path(temp: &TempDir) -> PathBuf {
        temp.path().join("data/devbox.sqlite3")
    }

    #[tokio::test]
    async fn managed_service_lifecycle_only_stops_the_devbox_owned_process_group() {
        let temp = TempDir::new().unwrap();
        let executable = fixture(&temp, "echo managed-service-ready; exec sleep 30");
        let core = Core::open(storage_path(&temp)).unwrap();
        let profiles = core
            .save_managed_service_profile(ManagedServiceProfile {
                id: String::new(),
                name: "Fixture Mailpit".into(),
                kind: "Email".into(),
                executable_path: executable.display().to_string(),
                arguments: Vec::new(),
                data_directory: None,
                ports: Vec::new(),
                created_at: 0,
            })
            .await
            .unwrap();
        let profile = profiles.first().unwrap();
        let started = core
            .start_managed_service(profile.id.clone())
            .await
            .unwrap();
        assert_eq!(started.status, "running");
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            core.managed_service_snapshots()[0]
                .logs
                .iter()
                .any(|line| line.contains("managed-service-ready"))
        );
        let restarted = core
            .restart_managed_service(profile.id.clone())
            .await
            .unwrap();
        assert_eq!(restarted.status, "running");
        core.stop_managed_service(profile.id.clone()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_ne!(core.managed_service_snapshots()[0].status, "running");
    }

    #[tokio::test]
    async fn managed_service_start_reports_an_external_port_conflict_before_spawning() {
        use std::net::TcpListener;
        let temp = TempDir::new().unwrap();
        let executable = fixture(&temp, "echo should-not-run; exec sleep 30");
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let core = Core::open(storage_path(&temp)).unwrap();
        let profiles = core
            .save_managed_service_profile(ManagedServiceProfile {
                id: String::new(),
                name: "Conflicted fixture".into(),
                kind: "Other".into(),
                executable_path: executable.display().to_string(),
                arguments: Vec::new(),
                data_directory: None,
                ports: vec![port],
                created_at: 0,
            })
            .await
            .unwrap();
        let error = core
            .start_managed_service(profiles[0].id.clone())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("already in use"));
        assert!(core.managed_service_snapshots().is_empty());
    }

    #[tokio::test]
    async fn shared_managed_service_stays_running_until_its_last_workspace_stops() {
        let temp = TempDir::new().unwrap();
        let executable = fixture(&temp, "exec sleep 30");
        let core = Core::open(storage_path(&temp)).unwrap();
        let profile = core
            .save_managed_service_profile(ManagedServiceProfile {
                id: String::new(),
                name: "Shared fixture".into(),
                kind: "Cache".into(),
                executable_path: executable.display().to_string(),
                arguments: Vec::new(),
                data_directory: None,
                ports: Vec::new(),
                created_at: 0,
            })
            .await
            .unwrap()
            .remove(0);
        let first = core
            .create_workspace("First".into())
            .await
            .unwrap()
            .remove(0);
        let second = core
            .create_workspace("Second".into())
            .await
            .unwrap()
            .into_iter()
            .find(|workspace| workspace.name == "Second")
            .unwrap();
        core.attach_service_to_workspace(first.id.clone(), profile.id.clone())
            .await
            .unwrap();
        core.attach_service_to_workspace(second.id.clone(), profile.id.clone())
            .await
            .unwrap();
        assert_eq!(
            core.storage.workspace_services(&second.id).unwrap(),
            std::slice::from_ref(&profile.id)
        );
        core.start_workspace(first.id.clone()).await.unwrap();
        assert_eq!(
            core.workspace_service_consumers.lock().unwrap()[&profile.id].len(),
            1
        );
        core.start_workspace(second.id.clone()).await.unwrap();
        assert_eq!(
            core.workspace_service_consumers.lock().unwrap()[&profile.id].len(),
            2
        );
        core.stop_workspace(first.id).await.unwrap();
        assert_eq!(core.managed_service_snapshots()[0].status, "running");
        core.stop_workspace(second.id).await.unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_ne!(core.managed_service_snapshots()[0].status, "running");
    }

    #[tokio::test]
    async fn workspace_dependency_failure_stops_projects_started_for_that_attempt() {
        let temp = TempDir::new().unwrap();
        let runtime = fixture(
            &temp,
            "if [ \"$1\" = \"--version\" ]; then echo 'Fixture 1.0'; else exec sleep 30; fi",
        );
        let core = Core::open(storage_path(&temp)).unwrap();
        let binary = core
            .save_custom_binary(
                "Fixture runtime".into(),
                BinaryCategory::Runtime,
                runtime.display().to_string(),
                vec!["--version".into()],
                None,
            )
            .await
            .unwrap()
            .remove(0);
        let prerequisite_directory = temp.path().join("prerequisite");
        let dependant_directory = temp.path().join("dependant");
        fs::create_dir(&prerequisite_directory).unwrap();
        fs::create_dir(&dependant_directory).unwrap();
        let prerequisite = core
            .add_project(
                "Prerequisite".into(),
                prerequisite_directory.display().to_string(),
                Some(binary.id),
                Vec::new(),
            )
            .await
            .unwrap()
            .remove(0);
        let dependant = core
            .add_project(
                "Dependant".into(),
                dependant_directory.display().to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap()
            .remove(0);
        let workspace = core
            .create_workspace("Failure recovery".into())
            .await
            .unwrap()
            .remove(0);
        core.add_project_to_workspace(workspace.id.clone(), dependant.id.clone())
            .await
            .unwrap();
        core.add_project_to_workspace(workspace.id.clone(), prerequisite.id.clone())
            .await
            .unwrap();
        core.set_workspace_dependency(
            workspace.id.clone(),
            dependant.id,
            prerequisite.id.clone(),
            true,
        )
        .await
        .unwrap();
        let error = core.start_workspace(workspace.id).await.unwrap_err();
        assert!(error.to_string().contains("recovery"));
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(
            !core
                .process_snapshots()
                .iter()
                .any(|process| process.project_id == prerequisite.id
                    && process.status == ProcessStatus::Running)
        );
    }

    #[tokio::test]
    async fn proxy_install_validates_first_and_preserves_the_previous_file() {
        let temp = TempDir::new().unwrap();
        let project_dir = temp.path().join("proxy-project");
        fs::create_dir(&project_dir).unwrap();
        let core = Core::open(storage_path(&temp)).unwrap();
        let project = core
            .add_project(
                "Proxy app".into(),
                project_dir.display().to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap()
            .remove(0);
        core.save_local_domain(LocalDomain {
            id: String::new(),
            project_id: project.id,
            domain: "api.localhost".into(),
            upstream: "http://127.0.0.1:8181".into(),
            websocket: false,
        })
        .await
        .unwrap();
        let caddy = fixture(
            &temp,
            "if [ \"$1\" = \"validate\" ]; then test -s \"$3\"; else exit 2; fi",
        );
        let target = temp.path().join("Caddyfile");
        fs::write(&target, "old config\n").unwrap();
        let result = core
            .apply_proxy_config(caddy.display().to_string(), target.display().to_string())
            .await
            .unwrap();
        assert!(result.contains("Reload Caddy"));
        assert!(
            fs::read_to_string(&target)
                .unwrap()
                .contains("api.localhost")
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("Caddyfile.devbox-backup")).unwrap(),
            "old config\n"
        );
        core.restore_proxy_config(target.display().to_string())
            .await
            .unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "old config\n");
    }

    #[tokio::test]
    async fn rejected_proxy_configuration_leaves_the_previous_file_untouched() {
        let temp = TempDir::new().unwrap();
        let caddy = fixture(&temp, "exit 4");
        let target = temp.path().join("Caddyfile");
        fs::write(&target, "known good config\n").unwrap();
        let error = proxy::install_caddyfile(&caddy, &target, "invalid candidate\n".into())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("rejected"));
        assert_eq!(fs::read_to_string(&target).unwrap(), "known good config\n");
        assert!(!temp.path().join("Caddyfile.devbox-backup").exists());
    }

    #[tokio::test]
    async fn extension_registry_requires_explicit_capability_approval_and_never_removes_manifests()
    {
        let temp = TempDir::new().unwrap();
        let manifest = temp.path().join("devbox-extension.json");
        fs::write(&manifest, r#"{"id":"example.readonly","name":"Read Only","version":"1.0","api_version":1,"capabilities":["projects.read","processes.read"]}"#).unwrap();
        let core = Core::open(storage_path(&temp)).unwrap();
        let extensions = core
            .register_extension(manifest.display().to_string())
            .await
            .unwrap();
        assert!(!extensions[0].enabled);
        assert!(extensions[0].approved_capabilities.is_empty());
        let denied = core
            .set_extension_permissions(extensions[0].id.clone(), Vec::new(), true)
            .await
            .unwrap_err();
        assert!(denied.to_string().contains("remains disabled"));
        assert!(!core.load_extensions().await.unwrap()[0].enabled);
        let extensions = core
            .approve_extension_and_enable(extensions[0].id.clone())
            .await
            .unwrap();
        assert!(extensions[0].enabled);
        assert_eq!(
            extensions[0].approved_capabilities,
            ["processes.read", "projects.read"]
        );
        core.disable_extension(extensions[0].id.clone())
            .await
            .unwrap();
        core.remove_extension(extensions[0].id.clone())
            .await
            .unwrap();
        assert!(manifest.exists());
    }

    #[test]
    fn parses_argument_lists_without_shell_expansion() {
        assert_eq!(
            parse_arguments("serve --port 8080"),
            ["serve", "--port", "8080"]
        );
        assert_eq!(parse_arguments("  "), Vec::<String>::new());
    }

    #[test]
    fn extracts_a_custom_version_regex_capture() {
        assert_eq!(
            extract_version("Runtime build v8.4.2 (stable)", Some(r"v([0-9.]+)")).unwrap(),
            Some("8.4.2".to_string())
        );
    }

    #[tokio::test]
    async fn validates_custom_binaries_and_persists_runtime_selection() {
        let temp = TempDir::new().unwrap();
        let executable = fixture(
            &temp,
            "if [ \"$1\" = \"--version\" ]; then echo 'Runtime 8.4.2'; fi",
        );
        let core = Core::open(storage_path(&temp)).unwrap();
        let probe = core
            .test_path(
                executable.display().to_string(),
                vec!["--version".into()],
                None,
            )
            .await;
        assert_eq!(probe.version.as_deref(), Some("8.4.2"));

        let binaries = core
            .save_custom_binary(
                "Fixture Runtime".into(),
                BinaryCategory::Runtime,
                executable.display().to_string(),
                vec!["--version".into()],
                None,
            )
            .await
            .unwrap();
        let registered = binaries
            .iter()
            .find(|binary| binary.display_name == "Fixture Runtime")
            .unwrap();
        assert_eq!(registered.version(), Some("8.4.2"));
        assert_eq!(registered.trust_status, "Available");
        let registered_id = registered.id.clone();

        let mut discovered = registered.clone();
        discovered.display_name = "Runtime from PATH".into();
        discovered.discovery_source = "PATH (php)".into();
        discovered.version_arguments = vec!["-v".into()];
        core.storage.save_discovered_binary(&discovered).unwrap();
        let preserved = core
            .load_binaries()
            .await
            .unwrap()
            .into_iter()
            .find(|binary| binary.id == registered_id)
            .unwrap();
        assert_eq!(preserved.display_name, "Fixture Runtime");
        assert_eq!(preserved.discovery_source, "Custom registration");
        assert_eq!(preserved.version_arguments, ["--version"]);

        let projects = core
            .add_project(
                "Example".into(),
                temp.path().display().to_string(),
                Some(registered_id.clone()),
                vec!["serve".into()],
            )
            .await
            .unwrap();
        assert_eq!(
            projects[0].runtime_binary_id.as_deref(),
            Some(registered_id.as_str())
        );
        assert_eq!(
            core.load_projects().await.unwrap()[0].startup_arguments,
            ["serve"]
        );

        core.save_dark_mode(true).await.unwrap();
        assert!(core.load_dark_mode().await.unwrap());
    }

    #[tokio::test]
    async fn persists_discovery_directories_and_respects_disabled_startup_rescan() {
        let temp = TempDir::new().unwrap();
        let path = storage_path(&temp);
        let search_directory = temp.path().join("extra-binaries");
        fs::create_dir(&search_directory).unwrap();
        let core = Core::open(&path).unwrap();
        let saved = core
            .save_discovery_settings(
                vec![
                    search_directory.display().to_string(),
                    search_directory.display().to_string(),
                ],
                false,
            )
            .await
            .unwrap();
        assert_eq!(saved.0.len(), 1);
        assert!(!saved.1);

        drop(core);
        let reopened = Core::open(path).unwrap();
        assert_eq!(
            reopened.load_discovery_settings().await.unwrap(),
            (saved.0, false)
        );
        assert!(reopened.initial_binaries().await.unwrap().is_empty());
        #[cfg(unix)]
        {
            let permissions = fs::metadata(storage_path(&temp)).unwrap().permissions();
            assert_eq!(permissions.mode() & 0o777, 0o600);
        }
    }

    #[tokio::test]
    async fn validates_project_environment_names_duplicates_and_newlines() {
        let temp = TempDir::new().unwrap();
        let core = Core::open(storage_path(&temp)).unwrap();
        let project_dir = temp.path().join("project");
        fs::create_dir(&project_dir).unwrap();
        for environment in [
            vec![EnvironmentVariable {
                key: "BAD-NAME".into(),
                value: "value".into(),
                secret: false,
            }],
            vec![
                EnvironmentVariable {
                    key: "DUPLICATE".into(),
                    value: "first".into(),
                    secret: false,
                },
                EnvironmentVariable {
                    key: "DUPLICATE".into(),
                    value: "second".into(),
                    secret: true,
                },
            ],
            vec![EnvironmentVariable {
                key: "MULTILINE".into(),
                value: "line one\nline two".into(),
                secret: true,
            }],
        ] {
            let error = core
                .add_project_with_environment(
                    "Invalid environment".into(),
                    project_dir.display().to_string(),
                    None,
                    Vec::new(),
                    environment,
                )
                .await
                .unwrap_err();
            assert!(!error.to_string().is_empty());
        }
        assert!(core.load_projects().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn inspects_php_extensions_and_configuration_from_the_selected_executable() {
        let temp = TempDir::new().unwrap();
        let executable = fixture(
            &temp,
            "if [ \"$1\" = \"--version\" ]; then echo 'PHP 8.3.4'; elif [ \"$1\" = \"-m\" ]; then printf '[PHP Modules]\\nCore\\nPDO\\nredis\\n[Zend Modules]\\nZend OPcache\\n'; elif [ \"$1\" = \"--ini\" ]; then printf 'Loaded Configuration File: /opt/php83/php.ini\\nAdditional .ini files parsed: /opt/php83/conf.d/redis.ini, /opt/php83/conf.d/xdebug.ini\\n'; fi",
        );
        let core = Core::open(storage_path(&temp)).unwrap();
        let binary = core
            .save_custom_binary(
                "PHP 8.3 custom".into(),
                BinaryCategory::Runtime,
                executable.display().to_string(),
                vec!["--version".into()],
                None,
            )
            .await
            .unwrap()
            .remove(0);
        let report = core.inspect_php_extensions(binary.id).await.unwrap();
        assert_eq!(
            report.loaded_extensions,
            ["Core", "PDO", "redis", "Zend OPcache"]
        );
        assert_eq!(
            report.loaded_configuration.as_deref(),
            Some("/opt/php83/php.ini")
        );
        assert_eq!(report.additional_ini_files.len(), 2);
    }

    #[tokio::test]
    async fn failed_custom_binary_probe_reports_unreadable_output_without_registering_it() {
        let temp = TempDir::new().unwrap();
        let executable = fixture(&temp, "echo 'Usage: runtime [command]'");
        let core = Core::open(storage_path(&temp)).unwrap();
        let result = core
            .test_path(
                executable.display().to_string(),
                vec!["--version".into()],
                None,
            )
            .await;
        assert!(result.version.is_none());
        assert!(result.error.unwrap().contains("recognizable version"));

        let save_result = core
            .save_custom_binary(
                "Unreadable Runtime".into(),
                BinaryCategory::Runtime,
                executable.display().to_string(),
                vec!["--version".into()],
                None,
            )
            .await;
        assert!(save_result.is_err());
        assert!(core.load_binaries().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn resolves_symlink_registration_and_reports_non_executable_files() {
        let temp = TempDir::new().unwrap();
        let executable = fixture(&temp, "echo 'Runtime 4.5.6'");
        let alias = temp.path().join("runtime-link");
        std::os::unix::fs::symlink(&executable, &alias).unwrap();
        let core = Core::open(storage_path(&temp)).unwrap();
        let binaries = core
            .save_custom_binary(
                "Linked Runtime".into(),
                BinaryCategory::Runtime,
                alias.display().to_string(),
                Vec::new(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            binaries[0].executable_path,
            executable.canonicalize().unwrap().display().to_string()
        );

        let non_executable = temp.path().join("not-executable");
        fs::write(&non_executable, "#!/bin/sh\necho 'Runtime 1.0.0'\n").unwrap();
        let mut permissions = fs::metadata(&non_executable).unwrap().permissions();
        permissions.set_mode(0o644);
        fs::set_permissions(&non_executable, permissions).unwrap();
        let result = core
            .test_path(non_executable.display().to_string(), Vec::new(), None)
            .await;
        assert!(result.error.unwrap().contains("executable permission"));
    }

    #[tokio::test]
    async fn removing_a_binary_keeps_the_file_and_persists_across_reopen() {
        let temp = TempDir::new().unwrap();
        let path = storage_path(&temp);
        let executable = fixture(&temp, "echo 'Runtime 1.0.0'");
        let core = Core::open(&path).unwrap();
        let binary = core
            .save_custom_binary(
                "Fixture Runtime".into(),
                BinaryCategory::Runtime,
                executable.display().to_string(),
                Vec::new(),
                None,
            )
            .await
            .unwrap()
            .remove(0);

        assert!(core.remove_binary(binary.id).await.unwrap().is_empty());
        assert!(
            executable.exists(),
            "removing registration must keep the file"
        );
        drop(core);
        let reopened = Core::open(path).unwrap();
        assert!(reopened.load_binaries().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn reports_deleted_project_and_runtime_paths_before_starting() {
        let temp = TempDir::new().unwrap();
        let executable = fixture(&temp, "echo 'Runtime 1.0.0'");
        let core = Core::open(storage_path(&temp)).unwrap();
        let binary = core
            .save_custom_binary(
                "Fixture Runtime".into(),
                BinaryCategory::Runtime,
                executable.display().to_string(),
                Vec::new(),
                None,
            )
            .await
            .unwrap()
            .remove(0);
        let project_dir = temp.path().join("project");
        fs::create_dir(&project_dir).unwrap();
        let project = core
            .add_project(
                "Deleted project".into(),
                project_dir.display().to_string(),
                Some(binary.id),
                Vec::new(),
            )
            .await
            .unwrap()
            .remove(0);
        fs::remove_dir(&project_dir).unwrap();
        let error = core.start_project(project.id.clone()).await.unwrap_err();
        assert!(error.to_string().contains("no longer accessible"));

        fs::create_dir(&project_dir).unwrap();
        fs::remove_file(&executable).unwrap();
        let error = core.start_project(project.id).await.unwrap_err();
        assert!(error.to_string().contains("Fixture Runtime is unavailable"));
        assert!(error.to_string().contains("cannot access executable"));
    }

    #[tokio::test]
    async fn reports_missing_projects_and_binary_registrations_for_followup_actions() {
        let temp = TempDir::new().unwrap();
        let path = storage_path(&temp);
        let executable = fixture(&temp, "echo 'Runtime 1.0.0'");
        let core = Core::open(&path).unwrap();
        let binary = core
            .save_custom_binary(
                "Fixture Runtime".into(),
                BinaryCategory::Runtime,
                executable.display().to_string(),
                Vec::new(),
                None,
            )
            .await
            .unwrap()
            .remove(0);
        let project_dir = temp.path().join("project");
        fs::create_dir(&project_dir).unwrap();
        let project = core
            .add_project(
                "Fixture project".into(),
                project_dir.display().to_string(),
                Some(binary.id.clone()),
                Vec::new(),
            )
            .await
            .unwrap()
            .remove(0);

        let error = core
            .assign_runtime(project.id.clone(), Some("missing-binary".into()))
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("runtime registration no longer exists")
        );
        core.remove_binary(binary.id.clone()).await.unwrap();
        let error = core
            .assign_runtime(project.id.clone(), Some(binary.id.clone()))
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("runtime registration no longer exists")
        );

        rusqlite::Connection::open(path)
            .unwrap()
            .execute("DELETE FROM projects WHERE id = ?1", [&project.id])
            .unwrap();
        let error = core.assign_runtime(project.id, None).await.unwrap_err();
        assert!(error.to_string().contains("project no longer exists"));
        let error = core.remove_binary(binary.id).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("binary registration no longer exists")
        );
    }

    #[tokio::test]
    async fn starts_captures_logs_and_stops_only_the_project_process_group() {
        let temp = TempDir::new().unwrap();
        let executable = fixture(
            &temp,
            "if [ \"$1\" = \"--version\" ]; then echo 'Runtime 1.2.3'; exit 0; fi\necho 'project started'\nexec sleep 60",
        );
        let core = Core::open(storage_path(&temp)).unwrap();
        let binaries = core
            .save_custom_binary(
                "Fixture Runtime".into(),
                BinaryCategory::Runtime,
                executable.display().to_string(),
                vec!["--version".into()],
                None,
            )
            .await
            .unwrap();
        let project_dir = temp.path().join("project");
        fs::create_dir(&project_dir).unwrap();
        let projects = core
            .add_project(
                "Process fixture".into(),
                project_dir.display().to_string(),
                Some(binaries[0].id.clone()),
                Vec::new(),
            )
            .await
            .unwrap();
        let started = core.start_project(projects[0].id.clone()).await.unwrap();
        assert!(started.pid > 0);

        let mut saw_output = false;
        for _ in 0..40 {
            if core.process_snapshots().iter().any(|process| {
                process
                    .logs
                    .iter()
                    .any(|line| line.contains("project started"))
            }) {
                saw_output = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(
            saw_output,
            "stdout should be streamed to the process snapshot"
        );

        core.stop_project(projects[0].id.clone()).await.unwrap();
        for _ in 0..40 {
            if core
                .process_snapshots()
                .iter()
                .any(|process| process.status == ProcessStatus::Exited)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("process group should stop and update its status");
    }

    #[tokio::test]
    async fn restarts_a_running_project_only_after_the_previous_group_exits() {
        let temp = TempDir::new().unwrap();
        let executable = fixture(
            &temp,
            "if [ \"$1\" = \"--version\" ]; then echo 'Runtime 1.2.3'; exit 0; fi\necho 'project started'\nexec sleep 60",
        );
        let path = storage_path(&temp);
        let core = Core::open(&path).unwrap();
        let binary = core
            .save_custom_binary(
                "Fixture Runtime".into(),
                BinaryCategory::Runtime,
                executable.display().to_string(),
                vec!["--version".into()],
                None,
            )
            .await
            .unwrap()
            .remove(0);
        let project_dir = temp.path().join("project");
        fs::create_dir(&project_dir).unwrap();
        let project = core
            .add_project(
                "Restart fixture".into(),
                project_dir.display().to_string(),
                Some(binary.id),
                Vec::new(),
            )
            .await
            .unwrap()
            .remove(0);
        let first = core.start_project(project.id.clone()).await.unwrap();
        let mut saw_metrics = false;
        for _ in 0..120 {
            if core.process_snapshots().iter().any(|process| {
                process.project_id == project.id
                    && process.cpu_percent.is_some()
                    && process.memory_bytes.is_some()
            }) {
                saw_metrics = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(
            saw_metrics,
            "Linux process metrics should be sampled asynchronously"
        );
        let second = core.restart_project(project.id.clone()).await.unwrap();
        assert_ne!(first.pid, second.pid);

        let snapshots = core.process_snapshots();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].pid, second.pid);
        assert_eq!(snapshots[0].status, ProcessStatus::Running);

        let connection = rusqlite::Connection::open(path).unwrap();
        let sessions: Vec<(u32, String)> = connection
            .prepare(
                "SELECT pid, status FROM process_sessions WHERE project_id = ?1 ORDER BY rowid",
            )
            .unwrap()
            .query_map([&project.id], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            sessions,
            [(first.pid, "exited".into()), (second.pid, "running".into())]
        );
        core.stop_project(project.id.clone()).await.unwrap();
    }

    #[tokio::test]
    async fn runs_two_projects_with_their_selected_runtimes_and_arguments() {
        let temp = TempDir::new().unwrap();
        let first_executable = fixture_named(
            &temp,
            "runtime-alpha",
            "if [ \"$1\" = \"--version\" ]; then echo 'Runtime 1.0.0'; exit 0; fi\nprintf 'runtime=alpha cwd=%s\\n' \"$PWD\"\nprintf 'profile=%s token=%s\\n' \"$APP_PROFILE\" \"$API_TOKEN\"\nprintf 'arg=<%s>\\n' \"$1\" \"$2\"\nexec sleep 60",
        );
        let second_executable = fixture_named(
            &temp,
            "runtime-beta",
            "if [ \"$1\" = \"--version\" ]; then echo 'Runtime 2.0.0'; exit 0; fi\nprintf 'runtime=beta cwd=%s\\n' \"$PWD\"\nprintf 'profile=%s token=%s\\n' \"$APP_PROFILE\" \"$API_TOKEN\"\nprintf 'arg=<%s>\\n' \"$1\" \"$2\"\nexec sleep 60",
        );
        let core = Core::open(storage_path(&temp)).unwrap();
        let first_binary = core
            .save_custom_binary(
                "Runtime Alpha".into(),
                BinaryCategory::Runtime,
                first_executable.display().to_string(),
                vec!["--version".into()],
                None,
            )
            .await
            .unwrap()
            .into_iter()
            .find(|binary| binary.display_name == "Runtime Alpha")
            .unwrap();
        let second_binary = core
            .save_custom_binary(
                "Runtime Beta".into(),
                BinaryCategory::Runtime,
                second_executable.display().to_string(),
                vec!["--version".into()],
                None,
            )
            .await
            .unwrap()
            .into_iter()
            .find(|binary| binary.display_name == "Runtime Beta")
            .unwrap();

        let first_dir = temp.path().join("alpha-project");
        let second_dir = temp.path().join("beta-project");
        fs::create_dir(&first_dir).unwrap();
        fs::create_dir(&second_dir).unwrap();
        let first_project = core
            .add_project_with_environment(
                "Alpha".into(),
                first_dir.display().to_string(),
                Some(first_binary.id),
                vec!["--message".into(), "hello world".into()],
                vec![
                    EnvironmentVariable {
                        key: "APP_PROFILE".into(),
                        value: "alpha".into(),
                        secret: false,
                    },
                    EnvironmentVariable {
                        key: "API_TOKEN".into(),
                        value: "alpha-secret".into(),
                        secret: true,
                    },
                ],
            )
            .await
            .unwrap()
            .into_iter()
            .find(|project| project.name == "Alpha")
            .unwrap();
        let second_project = core
            .add_project_with_environment(
                "Beta".into(),
                second_dir.display().to_string(),
                Some(second_binary.id),
                vec!["--message".into(), "hello world".into()],
                vec![
                    EnvironmentVariable {
                        key: "APP_PROFILE".into(),
                        value: "beta".into(),
                        secret: false,
                    },
                    EnvironmentVariable {
                        key: "API_TOKEN".into(),
                        value: "beta-secret".into(),
                        secret: true,
                    },
                ],
            )
            .await
            .unwrap()
            .into_iter()
            .find(|project| project.name == "Beta")
            .unwrap();

        core.start_project(first_project.id.clone()).await.unwrap();
        core.start_project(second_project.id.clone()).await.unwrap();
        assert!(core.start_project(first_project.id.clone()).await.is_err());

        let mut first_logs = None;
        let mut second_logs = None;
        for _ in 0..80 {
            let snapshots = core.process_snapshots();
            first_logs = snapshots
                .iter()
                .find(|process| process.project_id == first_project.id)
                .map(|process| process.logs.iter().cloned().collect::<Vec<_>>());
            second_logs = snapshots
                .iter()
                .find(|process| process.project_id == second_project.id)
                .map(|process| process.logs.iter().cloned().collect::<Vec<_>>());
            if first_logs.as_ref().is_some_and(|logs| logs.len() >= 3)
                && second_logs.as_ref().is_some_and(|logs| logs.len() >= 3)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        for (logs, runtime, directory) in [
            (first_logs.unwrap(), "alpha", first_dir),
            (second_logs.unwrap(), "beta", second_dir),
        ] {
            assert!(
                logs.iter()
                    .any(|line| line.contains(&format!("runtime={runtime}")))
            );
            assert!(
                logs.iter()
                    .any(|line| line.contains(&directory.display().to_string()))
            );
            assert!(logs.iter().any(|line| line.contains("arg=<--message>")));
            assert!(logs.iter().any(|line| line.contains("arg=<hello world>")));
            assert!(
                logs.iter()
                    .any(|line| { line.contains(&format!("profile={runtime} token=[REDACTED]")) })
            );
            assert!(
                !logs
                    .iter()
                    .any(|line| { line.contains("alpha-secret") || line.contains("beta-secret") })
            );
        }

        core.stop_project(first_project.id.clone()).await.unwrap();
        core.stop_project(second_project.id.clone()).await.unwrap();
        for _ in 0..40 {
            if core
                .process_snapshots()
                .iter()
                .all(|process| process.status == ProcessStatus::Exited)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("both project process groups should stop");
    }

    #[tokio::test]
    async fn reports_unassigned_and_disabled_runtimes_without_spawning() {
        let temp = TempDir::new().unwrap();
        let executable = fixture(
            &temp,
            "if [ \"$1\" = \"--version\" ]; then echo 'Runtime 1.2.3'; exit 0; fi\nexit 0",
        );
        let core = Core::open(storage_path(&temp)).unwrap();
        let project_dir = temp.path().join("project");
        fs::create_dir(&project_dir).unwrap();
        let project = core
            .add_project(
                "Unassigned".into(),
                project_dir.display().to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap()
            .remove(0);
        assert!(core.start_project(project.id.clone()).await.is_err());

        let binary = core
            .save_custom_binary(
                "Fixture Runtime".into(),
                BinaryCategory::Runtime,
                executable.display().to_string(),
                vec!["--version".into()],
                None,
            )
            .await
            .unwrap()
            .into_iter()
            .find(|binary| binary.display_name == "Fixture Runtime")
            .unwrap();
        core.assign_runtime(project.id.clone(), Some(binary.id.clone()))
            .await
            .unwrap();
        let mut binary = binary;
        binary.enabled = false;
        core.storage.save_binary(&binary).unwrap();
        assert!(core.start_project(project.id).await.is_err());
        assert!(core.process_snapshots().is_empty());
    }

    #[tokio::test]
    async fn records_nonzero_project_exit_as_failed() {
        let temp = TempDir::new().unwrap();
        let executable = fixture(
            &temp,
            "if [ \"$1\" = \"--version\" ]; then echo 'Runtime 1.2.3'; exit 0; fi\necho 'intentional failure' >&2\nexit 23",
        );
        let core = Core::open(storage_path(&temp)).unwrap();
        let binary = core
            .save_custom_binary(
                "Fixture Runtime".into(),
                BinaryCategory::Runtime,
                executable.display().to_string(),
                vec!["--version".into()],
                None,
            )
            .await
            .unwrap()
            .remove(0);
        let project_dir = temp.path().join("project");
        fs::create_dir(&project_dir).unwrap();
        let project = core
            .add_project(
                "Failing project".into(),
                project_dir.display().to_string(),
                Some(binary.id),
                Vec::new(),
            )
            .await
            .unwrap()
            .remove(0);
        core.start_project(project.id.clone()).await.unwrap();
        for _ in 0..80 {
            let process = core
                .process_snapshots()
                .into_iter()
                .find(|process| process.project_id == project.id)
                .unwrap();
            if process.status == ProcessStatus::Failed {
                assert_eq!(process.exit_code, Some(23));
                assert!(
                    process
                        .logs
                        .iter()
                        .any(|line| line.contains("intentional failure"))
                );
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("nonzero process exit should be recorded as failure");
    }

    #[tokio::test]
    async fn records_an_unexpectedly_missing_process_as_failed() {
        let temp = TempDir::new().unwrap();
        let executable = fixture(
            &temp,
            "if [ \"$1\" = \"--version\" ]; then echo 'Runtime 1.2.3'; exit 0; fi\nexec sleep 60",
        );
        let core = Core::open(storage_path(&temp)).unwrap();
        let binary = core
            .save_custom_binary(
                "Fixture Runtime".into(),
                BinaryCategory::Runtime,
                executable.display().to_string(),
                vec!["--version".into()],
                None,
            )
            .await
            .unwrap()
            .remove(0);
        let project_dir = temp.path().join("project");
        fs::create_dir(&project_dir).unwrap();
        let project = core
            .add_project(
                "Vanishing process".into(),
                project_dir.display().to_string(),
                Some(binary.id),
                Vec::new(),
            )
            .await
            .unwrap()
            .remove(0);
        let started = core.start_project(project.id.clone()).await.unwrap();
        nix::sys::signal::killpg(
            nix::unistd::Pid::from_raw(started.pid as i32),
            nix::sys::signal::Signal::SIGKILL,
        )
        .unwrap();

        for _ in 0..80 {
            let process = core
                .process_snapshots()
                .into_iter()
                .find(|process| process.project_id == project.id)
                .unwrap();
            if process.status == ProcessStatus::Failed {
                assert!(process.error_detail.is_some());
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("an externally terminated process should be marked failed");
    }

    #[tokio::test]
    async fn bounds_process_logs_and_keeps_the_newest_lines() {
        let temp = TempDir::new().unwrap();
        let executable = fixture(
            &temp,
            "if [ \"$1\" = \"--version\" ]; then echo 'Runtime 1.2.3'; exit 0; fi\ni=1\nwhile [ \"$i\" -le 520 ]; do echo \"line $i\"; i=$((i + 1)); done",
        );
        let core = Core::open(storage_path(&temp)).unwrap();
        let binary = core
            .save_custom_binary(
                "Fixture Runtime".into(),
                BinaryCategory::Runtime,
                executable.display().to_string(),
                vec!["--version".into()],
                None,
            )
            .await
            .unwrap()
            .remove(0);
        let project_dir = temp.path().join("project");
        fs::create_dir(&project_dir).unwrap();
        let project = core
            .add_project(
                "Verbose project".into(),
                project_dir.display().to_string(),
                Some(binary.id),
                Vec::new(),
            )
            .await
            .unwrap()
            .remove(0);
        core.start_project(project.id.clone()).await.unwrap();

        for _ in 0..100 {
            if let Some(process) = core
                .process_snapshots()
                .into_iter()
                .find(|process| process.project_id == project.id)
                && process.status == ProcessStatus::Exited
                && process.logs.len() == 500
            {
                assert!(process.logs.front().unwrap().contains("line 21"));
                assert!(process.logs.back().unwrap().contains("line 520"));
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("the process log should be bounded to its newest 500 lines");
    }

    #[tokio::test]
    async fn migrates_legacy_project_rows_and_marks_orphaned_processes_unknown() {
        let temp = TempDir::new().unwrap();
        let path = storage_path(&temp);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE detected_binaries (
                    id TEXT PRIMARY KEY, display_name TEXT NOT NULL, category TEXT NOT NULL,
                    executable_path TEXT NOT NULL UNIQUE, resolved_path TEXT,
                    version_arguments TEXT NOT NULL, version_parser TEXT NOT NULL,
                    version_regex TEXT, detected_version TEXT, manual_version TEXT,
                    discovery_source TEXT NOT NULL, trust_status TEXT NOT NULL,
                    enabled INTEGER NOT NULL DEFAULT 1, last_verified_at INTEGER
                 );
                 CREATE TABLE projects (
                    id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE,
                    runtime_binary_id TEXT REFERENCES detected_binaries(id) ON DELETE SET NULL,
                    startup_arguments TEXT NOT NULL DEFAULT '[]', created_at INTEGER NOT NULL
                 );
                 CREATE TABLE process_sessions (
                    id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                    pid INTEGER NOT NULL, started_at INTEGER NOT NULL,
                    status TEXT NOT NULL, exit_code INTEGER
                 );
                 INSERT INTO projects VALUES ('project-old', 'Legacy', '/legacy', NULL, '[]', 123);
                 INSERT INTO process_sessions VALUES ('session-old', 'project-old', 4321, 123, 'running', NULL);",
            )
            .unwrap();
        drop(connection);

        let core = Core::open(&path).unwrap();
        let project = core.load_projects().await.unwrap().remove(0);
        assert_eq!(project.id, "project-old");
        assert!(project.detected_frameworks.is_empty());

        let connection = rusqlite::Connection::open(path).unwrap();
        let status: String = connection
            .query_row(
                "SELECT status FROM process_sessions WHERE id = 'session-old'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(status, "unknown");
        assert_eq!(version, 5);
    }

    #[tokio::test]
    async fn upgrades_schema_version_two_without_losing_binary_or_project_data() {
        let temp = TempDir::new().unwrap();
        let path = storage_path(&temp);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE detected_binaries (
                    id TEXT PRIMARY KEY, display_name TEXT NOT NULL, category TEXT NOT NULL,
                    executable_path TEXT NOT NULL UNIQUE, resolved_path TEXT,
                    version_arguments TEXT NOT NULL, version_parser TEXT NOT NULL,
                    version_regex TEXT, detected_version TEXT, manual_version TEXT,
                    discovery_source TEXT NOT NULL, trust_status TEXT NOT NULL,
                    enabled INTEGER NOT NULL DEFAULT 1, last_verified_at INTEGER
                 );
                 CREATE TABLE projects (
                    id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE,
                    runtime_binary_id TEXT REFERENCES detected_binaries(id) ON DELETE SET NULL,
                    startup_arguments TEXT NOT NULL DEFAULT '[]',
                    detected_frameworks TEXT NOT NULL DEFAULT '[]', created_at INTEGER NOT NULL
                 );
                 CREATE TABLE process_sessions (
                    id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                    pid INTEGER NOT NULL, started_at INTEGER NOT NULL,
                    status TEXT NOT NULL, exit_code INTEGER
                 );
                 INSERT INTO detected_binaries VALUES (
                    'binary-v2', 'Legacy Node', 'Runtime', '/usr/bin/node', '/usr/bin/node',
                    '[\"--version\"]', 'auto', NULL, '22.1.0', NULL,
                    'PATH (node)', 'Available', 1, 100
                 );
                 INSERT INTO projects VALUES (
                    'project-v2', 'Legacy app', '/legacy-app', 'binary-v2', '[]', '[\"Nuxt\"]', 100
                 );
                 INSERT INTO process_sessions VALUES (
                    'session-v2', 'project-v2', 4321, 100, 'running', NULL
                 );
                 PRAGMA user_version = 2;",
            )
            .unwrap();
        drop(connection);

        let core = Core::open(&path).unwrap();
        let binaries = core.load_binaries().await.unwrap();
        assert_eq!(binaries[0].id, "binary-v2");
        assert_eq!(binaries[0].version(), Some("22.1.0"));
        assert_eq!(binaries[0].probe_error, None);
        let projects = core.load_projects().await.unwrap();
        assert_eq!(projects[0].detected_frameworks, ["Nuxt"]);

        let connection = rusqlite::Connection::open(path).unwrap();
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        let process_status: String = connection
            .query_row(
                "SELECT status FROM process_sessions WHERE id = 'session-v2'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, 5);
        assert_eq!(process_status, "unknown");
    }

    #[tokio::test]
    async fn upserts_a_project_by_canonical_path_and_persists_runtime_and_arguments() {
        let temp = TempDir::new().unwrap();
        let core = Core::open(storage_path(&temp)).unwrap();
        let first = core
            .add_project(
                "Original name".into(),
                temp.path().display().to_string(),
                None,
                vec!["serve".into()],
            )
            .await
            .unwrap()
            .remove(0);
        let second = core
            .add_project(
                "Updated name".into(),
                temp.path().join(".").display().to_string(),
                None,
                vec!["dev".into(), "--port".into(), "8080".into()],
            )
            .await
            .unwrap();
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].id, first.id);
        assert_eq!(second[0].name, "Updated name");
        assert_eq!(second[0].startup_arguments, ["dev", "--port", "8080"]);
    }

    #[test]
    fn rejects_non_executable_custom_files() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("not-executable");
        fs::write(&path, "plain text").unwrap();
        assert!(validate_executable(Path::new(&path)).is_err());
    }

    #[tokio::test]
    async fn service_scan_reports_server_roles_without_starting_services() {
        let services = scan_services().await;
        let ids = services
            .iter()
            .map(|service| service.id.as_str())
            .collect::<Vec<_>>();
        assert!(ids.contains(&"mysql"));
        assert!(ids.contains(&"postgresql"));
        assert!(ids.contains(&"redis"));
        assert!(ids.contains(&"mailpit"));
    }

    #[test]
    fn detects_frameworks_from_fixed_manifests_without_running_project_code() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("artisan"), "").unwrap();
        fs::write(
            temp.path().join("composer.json"),
            r#"{"require":{"laravel/framework":"^12.0"}}"#,
        )
        .unwrap();
        fs::write(
            temp.path().join("package.json"),
            r#"{"dependencies":{"next":"^15.0.0","@sveltejs/kit":"^2.0.0"}}"#,
        )
        .unwrap();

        let frameworks = detect_frameworks(temp.path());
        assert_eq!(frameworks, ["Laravel", "Next.js", "SvelteKit"]);
    }

    #[test]
    fn ignores_oversized_project_manifests() {
        let temp = TempDir::new().unwrap();
        fs::write(
            temp.path().join("package.json"),
            format!("{}next", " ".repeat(1024 * 1024)),
        )
        .unwrap();
        assert!(detect_frameworks(temp.path()).is_empty());
    }
}
