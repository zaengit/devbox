use devbox_core::{
    BinaryCategory, BinaryRecord, Core, EnvironmentVariable, ProbeResult, ProcessSnapshot,
    ProcessStatus, Project, ServiceSnapshot, ServiceState, parse_arguments,
};
use devbox_icons::{AppIcon, icon};
use iced::widget::{
    button, checkbox, column, container, pick_list, row, scrollable, text, text_input,
};
use iced::{
    Background, Border, Color, Element, Length, Subscription, Task, Theme, theme, time, window,
};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::time::Duration;

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .init();

    iced::application(App::new, update, view)
        .title(window_title)
        .theme(app_theme)
        .subscription(subscription)
        .window(window::Settings {
            size: iced::Size::new(1120.0, 760.0),
            min_size: Some(iced::Size::new(860.0, 600.0)),
            ..Default::default()
        })
        .run()
}

fn window_title(_app: &App) -> String {
    "Devbox".to_string()
}

fn app_theme(app: &App) -> Theme {
    app.theme()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Page {
    Dashboard,
    Projects,
    Workspaces,
    Binaries,
    Processes,
    Services,
    Networking,
    Extensions,
    Settings,
}

type InitialLoadResult = (
    Result<Vec<BinaryRecord>, String>,
    Result<Vec<Project>, String>,
    Result<bool, String>,
    Result<(Vec<String>, bool), String>,
    Vec<ServiceSnapshot>,
);
type CoreListsResult = (
    Result<Vec<devbox_core::ManagedServiceProfile>, String>,
    Result<Vec<devbox_core::Workspace>, String>,
    Result<Vec<devbox_core::ExtensionRecord>, String>,
    Result<Vec<devbox_core::LocalDomain>, String>,
);

impl Page {
    fn icon(self) -> AppIcon {
        match self {
            Self::Dashboard => AppIcon::Dashboard,
            Self::Projects => AppIcon::Projects,
            Self::Workspaces => AppIcon::Workspaces,
            Self::Binaries => AppIcon::Binary,
            Self::Processes => AppIcon::Logs,
            Self::Services => AppIcon::Services,
            Self::Networking => AppIcon::Globe,
            Self::Extensions => AppIcon::Extensions,
            Self::Settings => AppIcon::Settings,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Dashboard => "Dashboard",
            Self::Projects => "Projects",
            Self::Workspaces => "Workspaces",
            Self::Binaries => "Binary Manager",
            Self::Processes => "Processes & Logs",
            Self::Services => "Native Services",
            Self::Networking => "Local Domains",
            Self::Extensions => "Extensions",
            Self::Settings => "Settings",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Self::Dashboard => "Your native development environment at a glance.",
            Self::Projects => {
                "Run projects from their existing folders with selected host runtimes."
            }
            Self::Workspaces => "Start related projects together in dependency order.",
            Self::Binaries => "Discover installed runtimes or register a specific executable.",
            Self::Processes => "Review process status and captured output.",
            Self::Services => "Inspect host services and manage user-owned service processes.",
            Self::Networking => "Generate and validate local Caddy routes for project domains.",
            Self::Extensions => "Review extension manifests and approve requested capabilities.",
            Self::Settings => "Choose where Devbox looks for runtimes and when it rescans.",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RuntimeChoice {
    id: String,
    label: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProjectChoice {
    id: String,
    label: String,
}

impl fmt::Display for ProjectChoice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.label)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WorkspaceChoice {
    id: String,
    label: String,
}

impl fmt::Display for WorkspaceChoice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.label)
    }
}

#[derive(Default)]
struct ServiceForm {
    name: String,
    kind: String,
    executable: String,
    arguments: String,
    data_directory: String,
    ports: String,
}

#[derive(Default)]
struct WorkspaceForm {
    name: String,
    workspace: Option<WorkspaceChoice>,
    project: Option<ProjectChoice>,
    dependency_project: Option<ProjectChoice>,
    prerequisite: Option<ProjectChoice>,
    service: Option<String>,
}

#[derive(Default)]
struct DomainForm {
    project: Option<ProjectChoice>,
    domain: String,
    upstream: String,
    websocket: bool,
    caddy_executable: String,
    config_target: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LogStreamChoice {
    All,
    Stdout,
    Stderr,
    Devbox,
}

impl LogStreamChoice {
    const ALL: [Self; 4] = [Self::All, Self::Stdout, Self::Stderr, Self::Devbox];

    fn matches(self, line: &str) -> bool {
        match self {
            Self::All => true,
            Self::Stdout => line.starts_with("[stdout]"),
            Self::Stderr => line.starts_with("[stderr]"),
            Self::Devbox => line.starts_with("[devbox]"),
        }
    }
}

impl fmt::Display for LogStreamChoice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::All => "All output",
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
            Self::Devbox => "Devbox messages",
        })
    }
}

impl fmt::Display for RuntimeChoice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.label)
    }
}

#[derive(Default)]
struct BinaryForm {
    name: String,
    category: String,
    path: String,
    arguments: String,
    version_regex: String,
    probe: Option<(String, ProbeResult)>,
}

#[derive(Default)]
struct ProjectForm {
    name: String,
    path: String,
    arguments: String,
    runtime: Option<RuntimeChoice>,
    environment_key: String,
    environment_value: String,
    environment_secret: bool,
    environment_variables: Vec<EnvironmentVariable>,
}

struct ProjectConfigForm {
    project_id: String,
    runtime: Option<RuntimeChoice>,
    runtime_binary_id: Option<String>,
    arguments: String,
    environment_key: String,
    environment_value: String,
    environment_secret: bool,
    environment_variables: Vec<EnvironmentVariable>,
}

struct App {
    core: Option<Core>,
    startup_error: Option<String>,
    binaries: Vec<BinaryRecord>,
    projects: Vec<Project>,
    processes: Vec<ProcessSnapshot>,
    services: Vec<ServiceSnapshot>,
    managed_service_profiles: Vec<devbox_core::ManagedServiceProfile>,
    managed_service_states: Vec<devbox_core::ManagedServiceSnapshot>,
    workspaces: Vec<devbox_core::Workspace>,
    extensions: Vec<devbox_core::ExtensionRecord>,
    local_domains: Vec<devbox_core::LocalDomain>,
    page: Page,
    dark_mode: bool,
    busy: bool,
    status: Option<(bool, String)>,
    show_binary_form: bool,
    show_project_form: bool,
    binary_form: BinaryForm,
    project_form: ProjectForm,
    project_config_form: Option<ProjectConfigForm>,
    log_query: String,
    log_stream: LogStreamChoice,
    binary_search_directories: Vec<String>,
    directory_entry: String,
    automatic_rescan: bool,
    php_reports: HashMap<String, Result<devbox_core::PhpExtensionReport, String>>,
    php_inspections: HashSet<String>,
    service_form: ServiceForm,
    workspace_form: WorkspaceForm,
    extension_manifest_path: String,
    domain_form: DomainForm,
    proxy_preview: String,
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let (core, startup_error) = match Core::open_default() {
            Ok(core) => (Some(core), None),
            Err(error) => (None, Some(error.to_string())),
        };
        let mut app = Self {
            core,
            startup_error,
            binaries: Vec::new(),
            projects: Vec::new(),
            processes: Vec::new(),
            services: Vec::new(),
            managed_service_profiles: Vec::new(),
            managed_service_states: Vec::new(),
            workspaces: Vec::new(),
            extensions: Vec::new(),
            local_domains: Vec::new(),
            page: Page::Dashboard,
            dark_mode: false,
            busy: false,
            status: None,
            show_binary_form: false,
            show_project_form: false,
            binary_form: BinaryForm {
                category: "Runtime".into(),
                ..Default::default()
            },
            project_form: ProjectForm {
                environment_secret: true,
                ..Default::default()
            },
            project_config_form: None,
            log_query: String::new(),
            log_stream: LogStreamChoice::All,
            binary_search_directories: Vec::new(),
            directory_entry: String::new(),
            automatic_rescan: true,
            php_reports: HashMap::new(),
            php_inspections: HashSet::new(),
            service_form: ServiceForm {
                kind: "Database".into(),
                ..Default::default()
            },
            workspace_form: WorkspaceForm::default(),
            extension_manifest_path: String::new(),
            domain_form: DomainForm {
                upstream: "http://127.0.0.1:3000".into(),
                caddy_executable: "caddy".into(),
                ..Default::default()
            },
            proxy_preview: String::new(),
        };
        let task = if let Some(core) = app.core.clone() {
            app.busy = true;
            Task::perform(
                async move {
                    let binaries = core.initial_binaries().await;
                    let projects = core.load_projects().await;
                    let dark_mode = core.load_dark_mode().await;
                    let discovery_settings = core.load_discovery_settings().await;
                    let services = devbox_core::scan_services().await;
                    (
                        binaries.map_err(|error| error.to_string()),
                        projects.map_err(|error| error.to_string()),
                        dark_mode.map_err(|error| error.to_string()),
                        discovery_settings.map_err(|error| error.to_string()),
                        services,
                    )
                },
                Message::InitialLoaded,
            )
        } else {
            Task::none()
        };
        (app, task)
    }

    fn theme(&self) -> Theme {
        let palette = colors(self.dark_mode);
        Theme::custom(
            if self.dark_mode {
                "Devbox Dark"
            } else {
                "Devbox Light"
            },
            theme::Palette {
                background: palette.background,
                text: palette.text,
                primary: palette.accent,
                success: palette.success,
                warning: palette.warning,
                danger: palette.error,
            },
        )
    }
}

#[derive(Clone, Debug)]
enum Message {
    InitialLoaded(InitialLoadResult),
    CoreListsLoaded(CoreListsResult),
    Navigate(Page),
    ToggleTheme,
    ThemeSaved(Result<(), String>),
    ToggleBinaryForm,
    ToggleProjectForm,
    Refresh,
    Tick,
    KeyboardEvent(iced::keyboard::Event),
    BinaryNameChanged(String),
    BinaryCategoryChanged(String),
    BinaryPathChanged(String),
    PickBinaryPath,
    BinaryPathPicked(Option<String>),
    BinaryArgumentsChanged(String),
    BinaryRegexChanged(String),
    TestBinary,
    BinaryTested(String, ProbeResult),
    SaveBinary,
    BinarySaved(Result<Vec<BinaryRecord>, String>),
    RemoveBinary(String),
    BinaryScanCompleted((Result<Vec<BinaryRecord>, String>, Vec<ServiceSnapshot>)),
    BinaryRemoved(Result<Vec<BinaryRecord>, String>),
    InspectPhpExtensions(String),
    PhpExtensionsInspected(String, Result<devbox_core::PhpExtensionReport, String>),
    ProjectNameChanged(String),
    ProjectPathChanged(String),
    PickProjectFolder,
    ProjectFolderPicked(Option<String>),
    ProjectArgumentsChanged(String),
    EnvironmentKeyChanged(String),
    EnvironmentValueChanged(String),
    EnvironmentSecretChanged(bool),
    AddEnvironmentVariable,
    RemoveEnvironmentVariable(String),
    ProjectRuntimeSelected(RuntimeChoice),
    AddProject,
    ProjectAdded(Result<Vec<Project>, String>),
    OpenProjectConfig(String),
    CloseProjectConfig,
    ProjectConfigRuntimeSelected(RuntimeChoice),
    ProjectConfigUseHostDefault,
    ProjectConfigArgumentsChanged(String),
    ProjectConfigEnvironmentKeyChanged(String),
    ProjectConfigEnvironmentValueChanged(String),
    ProjectConfigEnvironmentSecretChanged(bool),
    AddProjectConfigEnvironmentVariable,
    RemoveProjectConfigEnvironmentVariable(String),
    SaveProjectConfig,
    ProjectConfigSaved(Result<Vec<Project>, String>),
    StartProject(String),
    ProjectStarted(Result<ProcessSnapshot, String>),
    StopProject(String),
    ProjectStopped(String, Result<(), String>),
    RestartProject(String),
    ProjectRestarted(Result<ProcessSnapshot, String>),
    LogQueryChanged(String),
    LogStreamChanged(LogStreamChoice),
    DiscoveryDirectoryInputChanged(String),
    PickDiscoveryDirectory,
    DiscoveryDirectoryPicked(Option<String>),
    AddDiscoveryDirectory,
    RemoveDiscoveryDirectory(String),
    AutomaticRescanChanged(bool),
    SaveDiscoverySettings,
    DiscoverySettingsSaved(Result<(Vec<String>, bool), String>),
    ClearStatus,
    ServiceNameChanged(String),
    ServiceKindChanged(String),
    ServiceExecutableChanged(String),
    ServiceArgumentsChanged(String),
    ServiceDataDirectoryChanged(String),
    ServicePortsChanged(String),
    PickServiceExecutable,
    ServiceExecutablePicked(Option<String>),
    PickServiceDataDirectory,
    ServiceDataDirectoryPicked(Option<String>),
    SaveServiceProfile,
    ServiceProfileSaved(Result<Vec<devbox_core::ManagedServiceProfile>, String>),
    RemoveServiceProfile(String),
    ManagedServiceProfilesChanged(Result<Vec<devbox_core::ManagedServiceProfile>, String>),
    StartManagedService(String),
    StopManagedService(String),
    RestartManagedService(String),
    ManagedServiceActionDone(Result<(), String>),
    WorkspaceNameChanged(String),
    WorkspaceSelected(WorkspaceChoice),
    WorkspaceProjectSelected(ProjectChoice),
    DependencyProjectSelected(ProjectChoice),
    PrerequisiteSelected(ProjectChoice),
    WorkspaceServiceSelected(String),
    AttachWorkspaceService,
    CreateWorkspace,
    WorkspaceCreated(Result<Vec<devbox_core::Workspace>, String>),
    AddWorkspaceProject,
    AddWorkspaceDependency,
    WorkspaceChanged(Result<Vec<devbox_core::Workspace>, String>),
    StartWorkspace(String),
    StopWorkspace(String),
    WorkspaceActionDone(Result<Vec<ProcessSnapshot>, String>),
    RegisterExtension,
    ExtensionManifestChanged(String),
    PickExtensionManifest,
    ExtensionManifestPicked(Option<String>),
    ExtensionsChanged(Result<Vec<devbox_core::ExtensionRecord>, String>),
    ApproveExtension(String),
    DisableExtension(String),
    RemoveExtension(String),
    DomainProjectSelected(ProjectChoice),
    DomainNameChanged(String),
    DomainUpstreamChanged(String),
    DomainWebsocketChanged(bool),
    CaddyExecutableChanged(String),
    ProxyTargetChanged(String),
    PickProxyTarget,
    ProxyTargetPicked(Option<String>),
    SaveDomain,
    DomainSaved(Result<Vec<devbox_core::LocalDomain>, String>),
    RemoveDomain(String),
    DomainsChanged(Result<Vec<devbox_core::LocalDomain>, String>),
    RenderProxyConfig,
    ProxyConfigRendered(Result<String, String>),
    ApplyProxyConfig,
    ProxyConfigApplied(Result<String, String>),
    RestoreProxyConfig,
    ProxyConfigRestored(Result<String, String>),
}

fn update(app: &mut App, message: Message) -> Task<Message> {
    match message {
        Message::InitialLoaded((binaries, projects, dark_mode, discovery_settings, services)) => {
            app.busy = false;
            match binaries {
                Ok(binaries) => app.binaries = binaries,
                Err(error) => set_status(app, false, error),
            }
            match projects {
                Ok(projects) => app.projects = projects,
                Err(error) => set_status(app, false, error),
            }
            match dark_mode {
                Ok(dark_mode) => app.dark_mode = dark_mode,
                Err(error) => set_status(app, false, error),
            }
            match discovery_settings {
                Ok((directories, automatic_rescan)) => {
                    app.binary_search_directories = directories;
                    app.automatic_rescan = automatic_rescan;
                }
                Err(error) => set_status(app, false, error),
            }
            app.services = services;
            if let Some(core) = app.core.clone() {
                Task::perform(
                    async move {
                        let profiles = core
                            .load_managed_service_profiles()
                            .await
                            .map_err(|error| error.to_string());
                        let workspaces = core.storage.clone();
                        let workspaces =
                            tokio::task::spawn_blocking(move || workspaces.list_workspaces())
                                .await
                                .map_err(|error| error.to_string())
                                .and_then(|result| result.map_err(|error| error.to_string()));
                        let extensions = core
                            .load_extensions()
                            .await
                            .map_err(|error| error.to_string());
                        let domains = core.storage.clone();
                        let domains =
                            tokio::task::spawn_blocking(move || domains.list_local_domains())
                                .await
                                .map_err(|error| error.to_string())
                                .and_then(|result| result.map_err(|error| error.to_string()));
                        (profiles, workspaces, extensions, domains)
                    },
                    Message::CoreListsLoaded,
                )
            } else {
                Task::none()
            }
        }
        Message::CoreListsLoaded((profiles, workspaces, extensions, domains)) => {
            if let Ok(profiles) = profiles {
                app.managed_service_profiles = profiles;
            }
            if let Ok(workspaces) = workspaces {
                app.workspaces = workspaces;
            }
            if let Ok(extensions) = extensions {
                app.extensions = extensions;
            }
            if let Ok(domains) = domains {
                app.local_domains = domains;
            }
            app.managed_service_states = app
                .core
                .as_ref()
                .map(Core::managed_service_snapshots)
                .unwrap_or_default();
            Task::none()
        }
        Message::Navigate(page) => {
            app.page = page;
            app.status = None;
            Task::none()
        }
        Message::ToggleTheme => {
            app.dark_mode = !app.dark_mode;
            if let Some(core) = app.core.clone() {
                let dark_mode = app.dark_mode;
                Task::perform(
                    async move {
                        core.save_dark_mode(dark_mode)
                            .await
                            .map_err(|error| error.to_string())
                    },
                    Message::ThemeSaved,
                )
            } else {
                Task::none()
            }
        }
        Message::ThemeSaved(result) => {
            if let Err(error) = result {
                set_status(
                    app,
                    false,
                    format!("Could not save appearance preference: {error}"),
                );
            }
            Task::none()
        }
        Message::ToggleBinaryForm => {
            app.show_binary_form = !app.show_binary_form;
            app.status = None;
            Task::none()
        }
        Message::ToggleProjectForm => {
            app.show_project_form = !app.show_project_form;
            app.status = None;
            Task::none()
        }
        Message::Refresh => rescan(app),
        Message::Tick => {
            if let Some(core) = &app.core {
                app.processes = core.process_snapshots();
                app.managed_service_states = core.managed_service_snapshots();
            }
            Task::none()
        }
        Message::KeyboardEvent(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab),
            modifiers,
            ..
        }) => {
            if modifiers.shift() {
                iced::widget::operation::focus_previous()
            } else {
                iced::widget::operation::focus_next()
            }
        }
        Message::KeyboardEvent(iced::keyboard::Event::KeyPressed { key, modifiers, .. })
            if modifiers.command() =>
        {
            app.page = match key.as_ref() {
                iced::keyboard::Key::Character("1") => Page::Dashboard,
                iced::keyboard::Key::Character("2") => Page::Projects,
                iced::keyboard::Key::Character("3") => Page::Workspaces,
                iced::keyboard::Key::Character("4") => Page::Binaries,
                iced::keyboard::Key::Character("5") => Page::Processes,
                iced::keyboard::Key::Character("6") => Page::Services,
                iced::keyboard::Key::Character("7") => Page::Networking,
                iced::keyboard::Key::Character("8") => Page::Extensions,
                iced::keyboard::Key::Character("9") => Page::Settings,
                _ => app.page,
            };
            app.status = None;
            Task::none()
        }
        Message::KeyboardEvent(_) => Task::none(),
        Message::BinaryNameChanged(value) => {
            app.binary_form.name = value;
            app.binary_form.probe = None;
            Task::none()
        }
        Message::BinaryCategoryChanged(value) => {
            app.binary_form.category = value;
            Task::none()
        }
        Message::BinaryPathChanged(value) => {
            app.binary_form.path = value;
            app.binary_form.probe = None;
            Task::none()
        }
        Message::PickBinaryPath => Task::perform(rfd::AsyncFileDialog::new().pick_file(), |file| {
            Message::BinaryPathPicked(file.map(|file| file.path().display().to_string()))
        }),
        Message::BinaryPathPicked(Some(path)) => {
            app.binary_form.path = path;
            app.binary_form.probe = None;
            Task::none()
        }
        Message::BinaryPathPicked(None) => Task::none(),
        Message::BinaryArgumentsChanged(value) => {
            app.binary_form.arguments = value;
            app.binary_form.probe = None;
            Task::none()
        }
        Message::BinaryRegexChanged(value) => {
            app.binary_form.version_regex = value;
            app.binary_form.probe = None;
            Task::none()
        }
        Message::TestBinary => {
            if app.binary_form.path.trim().is_empty() {
                set_status(app, false, "Choose an executable path first".into());
                return Task::none();
            }
            let (Some(core), path, arguments, regex) = (
                app.core.clone(),
                app.binary_form.path.trim().to_string(),
                parse_arguments(&app.binary_form.arguments),
                nonempty(&app.binary_form.version_regex),
            ) else {
                set_status(app, false, "The application database is unavailable".into());
                return Task::none();
            };
            app.busy = true;
            Task::perform(
                async move {
                    let result = core.test_path(path.clone(), arguments, regex).await;
                    (path, result)
                },
                |(path, result)| Message::BinaryTested(path, result),
            )
        }
        Message::BinaryTested(path, result) => {
            app.busy = false;
            app.binary_form.probe = Some((path, result.clone()));
            if let Some(version) = result.version {
                set_status(app, true, format!("Executable validated · {version}"));
            } else {
                set_status(
                    app,
                    false,
                    result
                        .error
                        .unwrap_or_else(|| "Version check failed".into()),
                );
            }
            Task::none()
        }
        Message::SaveBinary => save_binary(app),
        Message::BinarySaved(result) => {
            app.busy = false;
            match result {
                Ok(binaries) => {
                    app.binaries = binaries;
                    app.binary_form = BinaryForm {
                        category: "Runtime".into(),
                        ..Default::default()
                    };
                    app.show_binary_form = false;
                    set_status(app, true, "Binary registration saved".into());
                }
                Err(error) => set_status(app, false, error),
            }
            Task::none()
        }
        Message::BinaryScanCompleted((result, services)) => {
            app.busy = false;
            app.services = services;
            match result {
                Ok(binaries) => {
                    app.binaries = binaries;
                    set_status(
                        app,
                        true,
                        format!(
                            "Binary scan complete · {} registrations",
                            app.binaries.len()
                        ),
                    );
                }
                Err(error) => set_status(app, false, error),
            }
            Task::none()
        }
        Message::BinaryRemoved(result) => {
            app.busy = false;
            match result {
                Ok(binaries) => {
                    app.binaries = binaries;
                    set_status(app, true, "Binary registration removed".into());
                }
                Err(error) => set_status(app, false, error),
            }
            Task::none()
        }
        Message::InspectPhpExtensions(binary_id) => {
            let Some(core) = app.core.clone() else {
                return Task::none();
            };
            if !app.php_inspections.insert(binary_id.clone()) {
                return Task::none();
            }
            Task::perform(
                async move {
                    let result = core
                        .inspect_php_extensions(binary_id.clone())
                        .await
                        .map_err(|error| error.to_string());
                    (binary_id, result)
                },
                |(id, result)| Message::PhpExtensionsInspected(id, result),
            )
        }
        Message::PhpExtensionsInspected(binary_id, result) => {
            app.php_inspections.remove(&binary_id);
            if let Err(error) = &result {
                set_status(app, false, error.clone());
            }
            app.php_reports.insert(binary_id, result);
            Task::none()
        }
        Message::RemoveBinary(id) => {
            if let Some(core) = app.core.clone() {
                app.busy = true;
                Task::perform(
                    async move {
                        core.remove_binary(id)
                            .await
                            .map_err(|error| error.to_string())
                    },
                    Message::BinaryRemoved,
                )
            } else {
                Task::none()
            }
        }
        Message::ProjectNameChanged(value) => {
            app.project_form.name = value;
            Task::none()
        }
        Message::ProjectPathChanged(value) => {
            app.project_form.path = value;
            Task::none()
        }
        Message::PickProjectFolder => {
            Task::perform(rfd::AsyncFileDialog::new().pick_folder(), |folder| {
                Message::ProjectFolderPicked(
                    folder.map(|folder| folder.path().display().to_string()),
                )
            })
        }
        Message::ProjectFolderPicked(Some(path)) => {
            app.project_form.path = path;
            Task::none()
        }
        Message::ProjectFolderPicked(None) => Task::none(),
        Message::ProjectArgumentsChanged(value) => {
            app.project_form.arguments = value;
            Task::none()
        }
        Message::EnvironmentKeyChanged(value) => {
            app.project_form.environment_key = value;
            Task::none()
        }
        Message::EnvironmentValueChanged(value) => {
            app.project_form.environment_value = value;
            Task::none()
        }
        Message::EnvironmentSecretChanged(secret) => {
            app.project_form.environment_secret = secret;
            Task::none()
        }
        Message::AddEnvironmentVariable => {
            let key = app.project_form.environment_key.trim().to_string();
            let value = app.project_form.environment_value.clone();
            if key.is_empty() {
                set_status(app, false, "Enter an environment variable name".into());
            } else if app
                .project_form
                .environment_variables
                .iter()
                .any(|variable| variable.key == key)
            {
                set_status(
                    app,
                    false,
                    format!("{key} is already in this project profile"),
                );
            } else if value.is_empty() {
                set_status(
                    app,
                    false,
                    "Enter a value for this environment variable".into(),
                );
            } else {
                app.project_form
                    .environment_variables
                    .push(EnvironmentVariable {
                        key,
                        value,
                        secret: app.project_form.environment_secret,
                    });
                app.project_form.environment_key.clear();
                app.project_form.environment_value.clear();
                app.project_form.environment_secret = true;
            }
            Task::none()
        }
        Message::RemoveEnvironmentVariable(key) => {
            app.project_form
                .environment_variables
                .retain(|variable| variable.key != key);
            Task::none()
        }
        Message::ProjectRuntimeSelected(value) => {
            app.project_form.runtime = Some(value);
            Task::none()
        }
        Message::AddProject => add_project(app),
        Message::ProjectAdded(result) => {
            app.busy = false;
            match result {
                Ok(projects) => {
                    app.projects = projects;
                    app.project_form = ProjectForm::default();
                    app.project_form.environment_secret = true;
                    app.show_project_form = false;
                    set_status(app, true, "Project settings saved".into());
                }
                Err(error) => set_status(app, false, error),
            }
            Task::none()
        }
        Message::OpenProjectConfig(project_id) => {
            if app
                .project_config_form
                .as_ref()
                .is_some_and(|form| form.project_id == project_id)
            {
                app.project_config_form = None;
                app.status = None;
                return Task::none();
            }
            let choices = runtime_choices(&app.binaries);
            if let Some(project) = app.projects.iter().find(|p| p.id == project_id) {
                let runtime_binary_id = project.runtime_binary_id.clone();
                let runtime = runtime_binary_id
                    .as_ref()
                    .and_then(|id| choices.iter().find(|choice| &choice.id == id).cloned());
                app.project_config_form = Some(ProjectConfigForm {
                    project_id,
                    runtime,
                    runtime_binary_id,
                    arguments: project.startup_arguments.join(" "),
                    environment_key: String::new(),
                    environment_value: String::new(),
                    environment_secret: true,
                    environment_variables: project.environment_variables.clone(),
                });
                app.status = None;
            }
            Task::none()
        }
        Message::CloseProjectConfig => {
            app.project_config_form = None;
            app.status = None;
            Task::none()
        }
        Message::ProjectConfigRuntimeSelected(value) => {
            if let Some(form) = &mut app.project_config_form {
                form.runtime_binary_id = Some(value.id.clone());
                form.runtime = Some(value);
            }
            Task::none()
        }
        Message::ProjectConfigUseHostDefault => {
            if let Some(form) = &mut app.project_config_form {
                form.runtime_binary_id = None;
                form.runtime = None;
            }
            Task::none()
        }
        Message::ProjectConfigArgumentsChanged(value) => {
            if let Some(form) = &mut app.project_config_form {
                form.arguments = value;
            }
            Task::none()
        }
        Message::ProjectConfigEnvironmentKeyChanged(value) => {
            if let Some(form) = &mut app.project_config_form {
                form.environment_key = value;
            }
            Task::none()
        }
        Message::ProjectConfigEnvironmentValueChanged(value) => {
            if let Some(form) = &mut app.project_config_form {
                form.environment_value = value;
            }
            Task::none()
        }
        Message::ProjectConfigEnvironmentSecretChanged(secret) => {
            if let Some(form) = &mut app.project_config_form {
                form.environment_secret = secret;
            }
            Task::none()
        }
        Message::AddProjectConfigEnvironmentVariable => {
            let validation = if let Some(form) = &mut app.project_config_form {
                let key = form.environment_key.trim().to_string();
                let value = form.environment_value.clone();
                if key.is_empty() {
                    Err("Enter an environment variable name".to_string())
                } else if form
                    .environment_variables
                    .iter()
                    .any(|variable| variable.key == key)
                {
                    Err(format!("{key} is already in this project profile"))
                } else if value.is_empty() {
                    Err("Enter a value for this environment variable".to_string())
                } else {
                    form.environment_variables.push(EnvironmentVariable {
                        key,
                        value,
                        secret: form.environment_secret,
                    });
                    form.environment_key.clear();
                    form.environment_value.clear();
                    form.environment_secret = true;
                    Ok(())
                }
            } else {
                return Task::none();
            };
            match validation {
                Ok(()) => app.status = None,
                Err(error) => set_status(app, false, error),
            }
            Task::none()
        }
        Message::RemoveProjectConfigEnvironmentVariable(key) => {
            if let Some(form) = &mut app.project_config_form {
                form.environment_variables
                    .retain(|variable| variable.key != key);
            }
            Task::none()
        }
        Message::SaveProjectConfig => save_project_configuration(app),
        Message::ProjectConfigSaved(result) => {
            app.busy = false;
            match result {
                Ok(projects) => {
                    app.projects = projects;
                    app.project_config_form = None;
                    set_status(
                        app,
                        true,
                        "Project configuration saved · changes apply on next start".into(),
                    );
                }
                Err(error) => set_status(app, false, error),
            }
            Task::none()
        }
        Message::StartProject(id) => {
            if let Some(core) = app.core.clone() {
                app.busy = true;
                Task::perform(
                    async move {
                        core.start_project(id)
                            .await
                            .map_err(|error| error.to_string())
                    },
                    Message::ProjectStarted,
                )
            } else {
                Task::none()
            }
        }
        Message::ProjectStarted(result) => {
            app.busy = false;
            match result {
                Ok(snapshot) => {
                    app.processes = app
                        .core
                        .as_ref()
                        .map(Core::process_snapshots)
                        .unwrap_or_default();
                    app.page = Page::Processes;
                    set_status(
                        app,
                        true,
                        format!(
                            "Started {} as process {}",
                            snapshot.project_name, snapshot.pid
                        ),
                    );
                }
                Err(error) => set_status(app, false, error),
            }
            Task::none()
        }
        Message::StopProject(project_id) => {
            if let Some(core) = app.core.clone() {
                app.busy = true;
                let id = project_id.clone();
                Task::perform(
                    async move {
                        (
                            id,
                            core.stop_project(project_id)
                                .await
                                .map_err(|error| error.to_string()),
                        )
                    },
                    |(id, result)| Message::ProjectStopped(id, result),
                )
            } else {
                Task::none()
            }
        }
        Message::ProjectStopped(project_id, result) => {
            app.busy = false;
            app.processes = app
                .core
                .as_ref()
                .map(Core::process_snapshots)
                .unwrap_or_default();
            match result {
                Ok(()) => set_status(
                    app,
                    true,
                    format!("Shutdown requested for project {project_id}"),
                ),
                Err(error) => set_status(app, false, error),
            }
            Task::none()
        }
        Message::RestartProject(project_id) => {
            if let Some(core) = app.core.clone() {
                app.busy = true;
                Task::perform(
                    async move {
                        core.restart_project(project_id)
                            .await
                            .map_err(|error| error.to_string())
                    },
                    Message::ProjectRestarted,
                )
            } else {
                Task::none()
            }
        }
        Message::ProjectRestarted(result) => {
            app.busy = false;
            app.processes = app
                .core
                .as_ref()
                .map(Core::process_snapshots)
                .unwrap_or_default();
            match result {
                Ok(snapshot) => {
                    app.page = Page::Processes;
                    set_status(
                        app,
                        true,
                        format!(
                            "Restarted {} as process {}",
                            snapshot.project_name, snapshot.pid
                        ),
                    );
                }
                Err(error) => set_status(app, false, error),
            }
            Task::none()
        }
        Message::LogQueryChanged(query) => {
            app.log_query = query;
            Task::none()
        }
        Message::LogStreamChanged(stream) => {
            app.log_stream = stream;
            Task::none()
        }
        Message::DiscoveryDirectoryInputChanged(value) => {
            app.directory_entry = value;
            Task::none()
        }
        Message::PickDiscoveryDirectory => {
            Task::perform(rfd::AsyncFileDialog::new().pick_folder(), |folder| {
                Message::DiscoveryDirectoryPicked(
                    folder.map(|folder| folder.path().display().to_string()),
                )
            })
        }
        Message::DiscoveryDirectoryPicked(Some(path)) => {
            app.directory_entry = path;
            Task::none()
        }
        Message::DiscoveryDirectoryPicked(None) => Task::none(),
        Message::AddDiscoveryDirectory => {
            let directory = app.directory_entry.trim().to_string();
            if directory.is_empty() {
                set_status(app, false, "Choose a search directory first".into());
            } else if app.binary_search_directories.contains(&directory) {
                set_status(app, false, "That search directory is already listed".into());
            } else if app.binary_search_directories.len() >= 64 {
                set_status(app, false, "You can add up to 64 search directories".into());
            } else {
                app.binary_search_directories.push(directory);
                app.directory_entry.clear();
            }
            Task::none()
        }
        Message::RemoveDiscoveryDirectory(directory) => {
            app.binary_search_directories
                .retain(|existing| existing != &directory);
            Task::none()
        }
        Message::AutomaticRescanChanged(enabled) => {
            app.automatic_rescan = enabled;
            Task::none()
        }
        Message::SaveDiscoverySettings => save_discovery_settings(app),
        Message::DiscoverySettingsSaved(result) => {
            app.busy = false;
            match result {
                Ok((directories, automatic_rescan)) => {
                    app.binary_search_directories = directories;
                    app.automatic_rescan = automatic_rescan;
                    app.directory_entry.clear();
                    set_status(app, true, "Discovery settings saved".into());
                }
                Err(error) => set_status(app, false, error),
            }
            Task::none()
        }
        Message::ServiceNameChanged(value) => {
            app.service_form.name = value;
            Task::none()
        }
        Message::ServiceKindChanged(value) => {
            app.service_form.kind = value;
            Task::none()
        }
        Message::ServiceExecutableChanged(value) => {
            app.service_form.executable = value;
            Task::none()
        }
        Message::ServiceArgumentsChanged(value) => {
            app.service_form.arguments = value;
            Task::none()
        }
        Message::ServiceDataDirectoryChanged(value) => {
            app.service_form.data_directory = value;
            Task::none()
        }
        Message::ServicePortsChanged(value) => {
            app.service_form.ports = value;
            Task::none()
        }
        Message::PickServiceExecutable => {
            Task::perform(rfd::AsyncFileDialog::new().pick_file(), |file| {
                Message::ServiceExecutablePicked(file.map(|file| file.path().display().to_string()))
            })
        }
        Message::ServiceExecutablePicked(Some(path)) => {
            app.service_form.executable = path;
            Task::none()
        }
        Message::ServiceExecutablePicked(None) => Task::none(),
        Message::PickServiceDataDirectory => {
            Task::perform(rfd::AsyncFileDialog::new().pick_folder(), |folder| {
                Message::ServiceDataDirectoryPicked(
                    folder.map(|folder| folder.path().display().to_string()),
                )
            })
        }
        Message::ServiceDataDirectoryPicked(Some(path)) => {
            app.service_form.data_directory = path;
            Task::none()
        }
        Message::ServiceDataDirectoryPicked(None) => Task::none(),
        Message::SaveServiceProfile => {
            let ports = app
                .service_form
                .ports
                .split(',')
                .filter_map(|port| {
                    let port = port.trim();
                    if port.is_empty() {
                        None
                    } else {
                        port.parse::<u16>().ok()
                    }
                })
                .collect::<Vec<_>>();
            if app
                .service_form
                .ports
                .split(',')
                .any(|port| !port.trim().is_empty() && port.trim().parse::<u16>().is_err())
            {
                set_status(app, false, "Ports must be comma-separated numbers".into());
                return Task::none();
            }
            let profile = devbox_core::ManagedServiceProfile {
                id: String::new(),
                name: app.service_form.name.trim().into(),
                kind: app.service_form.kind.trim().into(),
                executable_path: app.service_form.executable.trim().into(),
                arguments: parse_arguments(&app.service_form.arguments),
                data_directory: nonempty(&app.service_form.data_directory),
                ports,
                created_at: 0,
            };
            let Some(core) = app.core.clone() else {
                return Task::none();
            };
            app.busy = true;
            Task::perform(
                async move {
                    core.save_managed_service_profile(profile)
                        .await
                        .map_err(|error| error.to_string())
                },
                Message::ServiceProfileSaved,
            )
        }
        Message::ServiceProfileSaved(result) | Message::ManagedServiceProfilesChanged(result) => {
            app.busy = false;
            match result {
                Ok(profiles) => {
                    app.managed_service_profiles = profiles;
                    set_status(app, true, "Managed service profile saved".into());
                }
                Err(error) => set_status(app, false, error),
            }
            Task::none()
        }
        Message::RemoveServiceProfile(id) => {
            let Some(core) = app.core.clone() else {
                return Task::none();
            };
            app.busy = true;
            Task::perform(
                async move {
                    core.remove_managed_service_profile(id)
                        .await
                        .map_err(|error| error.to_string())
                },
                Message::ManagedServiceProfilesChanged,
            )
        }
        Message::StartManagedService(id) => {
            let Some(core) = app.core.clone() else {
                return Task::none();
            };
            app.busy = true;
            Task::perform(
                async move {
                    core.start_managed_service(id)
                        .await
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                },
                Message::ManagedServiceActionDone,
            )
        }
        Message::StopManagedService(id) => {
            let Some(core) = app.core.clone() else {
                return Task::none();
            };
            app.busy = true;
            Task::perform(
                async move {
                    core.stop_managed_service(id)
                        .await
                        .map_err(|error| error.to_string())
                },
                Message::ManagedServiceActionDone,
            )
        }
        Message::RestartManagedService(id) => {
            let Some(core) = app.core.clone() else {
                return Task::none();
            };
            app.busy = true;
            Task::perform(
                async move {
                    core.restart_managed_service(id)
                        .await
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                },
                Message::ManagedServiceActionDone,
            )
        }
        Message::ManagedServiceActionDone(result) => {
            app.busy = false;
            app.managed_service_states = app
                .core
                .as_ref()
                .map(Core::managed_service_snapshots)
                .unwrap_or_default();
            match result {
                Ok(()) => set_status(app, true, "Devbox-managed service action completed".into()),
                Err(error) => set_status(app, false, error),
            }
            Task::none()
        }
        Message::WorkspaceNameChanged(value) => {
            app.workspace_form.name = value;
            Task::none()
        }
        Message::WorkspaceSelected(value) => {
            app.workspace_form.workspace = Some(value);
            Task::none()
        }
        Message::WorkspaceProjectSelected(value) => {
            app.workspace_form.project = Some(value);
            Task::none()
        }
        Message::DependencyProjectSelected(value) => {
            app.workspace_form.dependency_project = Some(value);
            Task::none()
        }
        Message::PrerequisiteSelected(value) => {
            app.workspace_form.prerequisite = Some(value);
            Task::none()
        }
        Message::WorkspaceServiceSelected(value) => {
            app.workspace_form.service = Some(value);
            Task::none()
        }
        Message::AttachWorkspaceService => {
            let (Some(workspace), Some(service), Some(core)) = (
                app.workspace_form.workspace.clone(),
                app.workspace_form.service.clone(),
                app.core.clone(),
            ) else {
                set_status(
                    app,
                    false,
                    "Select a workspace and a managed service".into(),
                );
                return Task::none();
            };
            app.busy = true;
            Task::perform(
                async move {
                    core.attach_service_to_workspace(workspace.id, service)
                        .await
                        .map_err(|error| error.to_string())
                },
                Message::WorkspaceChanged,
            )
        }
        Message::CreateWorkspace => {
            let name = app.workspace_form.name.trim().to_string();
            if name.is_empty() {
                set_status(app, false, "Enter a workspace name".into());
                return Task::none();
            }
            let Some(core) = app.core.clone() else {
                return Task::none();
            };
            app.busy = true;
            Task::perform(
                async move {
                    core.create_workspace(name)
                        .await
                        .map_err(|error| error.to_string())
                },
                Message::WorkspaceCreated,
            )
        }
        Message::WorkspaceCreated(result) | Message::WorkspaceChanged(result) => {
            app.busy = false;
            match result {
                Ok(workspaces) => {
                    app.workspaces = workspaces;
                    set_status(app, true, "Workspace updated".into());
                }
                Err(error) => set_status(app, false, error),
            }
            Task::none()
        }
        Message::AddWorkspaceProject => {
            let (Some(workspace), Some(project), Some(core)) = (
                app.workspace_form.workspace.clone(),
                app.workspace_form.project.clone(),
                app.core.clone(),
            ) else {
                set_status(app, false, "Select a workspace and a project".into());
                return Task::none();
            };
            app.busy = true;
            Task::perform(
                async move {
                    core.add_project_to_workspace(workspace.id, project.id)
                        .await
                        .map_err(|error| error.to_string())
                },
                Message::WorkspaceChanged,
            )
        }
        Message::AddWorkspaceDependency => {
            let (Some(workspace), Some(project), Some(prerequisite), Some(core)) = (
                app.workspace_form.workspace.clone(),
                app.workspace_form.dependency_project.clone(),
                app.workspace_form.prerequisite.clone(),
                app.core.clone(),
            ) else {
                set_status(
                    app,
                    false,
                    "Select a workspace, dependent project, and prerequisite".into(),
                );
                return Task::none();
            };
            app.busy = true;
            Task::perform(
                async move {
                    core.set_workspace_dependency(workspace.id, project.id, prerequisite.id, true)
                        .await
                        .map_err(|error| error.to_string())
                },
                Message::WorkspaceChanged,
            )
        }
        Message::StartWorkspace(id) => {
            let Some(core) = app.core.clone() else {
                return Task::none();
            };
            app.busy = true;
            Task::perform(
                async move {
                    core.start_workspace(id)
                        .await
                        .map_err(|error| error.to_string())
                },
                Message::WorkspaceActionDone,
            )
        }
        Message::StopWorkspace(id) => {
            let Some(core) = app.core.clone() else {
                return Task::none();
            };
            app.busy = true;
            Task::perform(
                async move {
                    core.stop_workspace(id)
                        .await
                        .map_err(|error| error.to_string())
                        .map(|_| core.process_snapshots())
                },
                Message::WorkspaceActionDone,
            )
        }
        Message::WorkspaceActionDone(result) => {
            app.busy = false;
            app.processes = app
                .core
                .as_ref()
                .map(Core::process_snapshots)
                .unwrap_or_default();
            match result {
                Ok(_) => set_status(app, true, "Workspace process action completed".into()),
                Err(error) => set_status(app, false, error),
            }
            Task::none()
        }
        Message::RegisterExtension => {
            let path = app.extension_manifest_path.trim().to_string();
            if path.is_empty() {
                set_status(app, false, "Choose a devbox-extension.json manifest".into());
                return Task::none();
            }
            let Some(core) = app.core.clone() else {
                return Task::none();
            };
            app.busy = true;
            Task::perform(
                async move {
                    core.register_extension(path)
                        .await
                        .map_err(|error| error.to_string())
                },
                Message::ExtensionsChanged,
            )
        }
        Message::ExtensionManifestChanged(value) => {
            app.extension_manifest_path = value;
            Task::none()
        }
        Message::PickExtensionManifest => {
            Task::perform(rfd::AsyncFileDialog::new().pick_file(), |file| {
                Message::ExtensionManifestPicked(file.map(|file| file.path().display().to_string()))
            })
        }
        Message::ExtensionManifestPicked(Some(path)) => {
            app.extension_manifest_path = path;
            Task::none()
        }
        Message::ExtensionManifestPicked(None) => Task::none(),
        Message::ApproveExtension(id) => {
            let Some(core) = app.core.clone() else {
                return Task::none();
            };
            app.busy = true;
            Task::perform(
                async move {
                    core.approve_extension_and_enable(id)
                        .await
                        .map_err(|error| error.to_string())
                },
                Message::ExtensionsChanged,
            )
        }
        Message::DisableExtension(id) => {
            let Some(core) = app.core.clone() else {
                return Task::none();
            };
            app.busy = true;
            Task::perform(
                async move {
                    core.disable_extension(id)
                        .await
                        .map_err(|error| error.to_string())
                },
                Message::ExtensionsChanged,
            )
        }
        Message::RemoveExtension(id) => {
            let Some(core) = app.core.clone() else {
                return Task::none();
            };
            app.busy = true;
            Task::perform(
                async move {
                    core.remove_extension(id)
                        .await
                        .map_err(|error| error.to_string())
                },
                Message::ExtensionsChanged,
            )
        }
        Message::ExtensionsChanged(result) => {
            app.busy = false;
            match result {
                Ok(extensions) => {
                    app.extensions = extensions;
                    set_status(
                        app,
                        true,
                        "Extension registry updated; no extension code was executed".into(),
                    );
                }
                Err(error) => set_status(app, false, error),
            }
            Task::none()
        }
        Message::DomainProjectSelected(value) => {
            app.domain_form.project = Some(value);
            Task::none()
        }
        Message::DomainNameChanged(value) => {
            app.domain_form.domain = value;
            Task::none()
        }
        Message::DomainUpstreamChanged(value) => {
            app.domain_form.upstream = value;
            Task::none()
        }
        Message::DomainWebsocketChanged(value) => {
            app.domain_form.websocket = value;
            Task::none()
        }
        Message::CaddyExecutableChanged(value) => {
            app.domain_form.caddy_executable = value;
            Task::none()
        }
        Message::ProxyTargetChanged(value) => {
            app.domain_form.config_target = value;
            Task::none()
        }
        Message::PickProxyTarget => Task::perform(
            rfd::AsyncFileDialog::new()
                .set_file_name("Caddyfile")
                .save_file(),
            |file| Message::ProxyTargetPicked(file.map(|file| file.path().display().to_string())),
        ),
        Message::ProxyTargetPicked(Some(path)) => {
            app.domain_form.config_target = path;
            Task::none()
        }
        Message::ProxyTargetPicked(None) => Task::none(),
        Message::SaveDomain => {
            let (Some(project), Some(core)) = (app.domain_form.project.clone(), app.core.clone())
            else {
                set_status(app, false, "Select a project for this domain".into());
                return Task::none();
            };
            let route = devbox_core::LocalDomain {
                id: String::new(),
                project_id: project.id,
                domain: app.domain_form.domain.trim().into(),
                upstream: app.domain_form.upstream.trim().into(),
                websocket: app.domain_form.websocket,
            };
            app.busy = true;
            Task::perform(
                async move {
                    core.save_local_domain(route)
                        .await
                        .map_err(|error| error.to_string())
                },
                Message::DomainSaved,
            )
        }
        Message::DomainSaved(result) => {
            app.busy = false;
            match result {
                Ok(domains) => {
                    app.local_domains = domains;
                    set_status(app, true, "Local domain saved".into());
                }
                Err(error) => set_status(app, false, error),
            }
            Task::none()
        }
        Message::RemoveDomain(id) => {
            let Some(core) = app.core.clone() else {
                return Task::none();
            };
            app.busy = true;
            Task::perform(
                async move {
                    core.remove_local_domain(id)
                        .await
                        .map_err(|error| error.to_string())
                },
                Message::DomainsChanged,
            )
        }
        Message::DomainsChanged(result) => {
            app.busy = false;
            match result {
                Ok(domains) => {
                    app.local_domains = domains;
                    set_status(app, true, "Local domain removed".into());
                }
                Err(error) => set_status(app, false, error),
            }
            Task::none()
        }
        Message::RenderProxyConfig => {
            let Some(core) = app.core.clone() else {
                return Task::none();
            };
            Task::perform(
                async move {
                    core.render_proxy_config()
                        .await
                        .map_err(|error| error.to_string())
                },
                Message::ProxyConfigRendered,
            )
        }
        Message::ProxyConfigRendered(result) => {
            match result {
                Ok(config) => app.proxy_preview = config,
                Err(error) => set_status(app, false, error),
            }
            Task::none()
        }
        Message::ApplyProxyConfig => {
            let executable = app.domain_form.caddy_executable.trim().to_string();
            let target = app.domain_form.config_target.trim().to_string();
            if executable.is_empty() || target.is_empty() {
                set_status(
                    app,
                    false,
                    "Choose a Caddy executable and a user-owned config path".into(),
                );
                return Task::none();
            }
            let Some(core) = app.core.clone() else {
                return Task::none();
            };
            app.busy = true;
            Task::perform(
                async move {
                    core.apply_proxy_config(executable, target)
                        .await
                        .map_err(|error| error.to_string())
                },
                Message::ProxyConfigApplied,
            )
        }
        Message::ProxyConfigApplied(result) => {
            app.busy = false;
            match result {
                Ok(message) => set_status(app, true, message),
                Err(error) => set_status(app, false, error),
            }
            Task::none()
        }
        Message::RestoreProxyConfig => {
            let target = app.domain_form.config_target.trim().to_string();
            if target.is_empty() {
                set_status(
                    app,
                    false,
                    "Choose the Caddyfile whose previous version you want to restore".into(),
                );
                return Task::none();
            }
            let Some(core) = app.core.clone() else {
                return Task::none();
            };
            app.busy = true;
            Task::perform(
                async move {
                    core.restore_proxy_config(target)
                        .await
                        .map_err(|error| error.to_string())
                },
                Message::ProxyConfigRestored,
            )
        }
        Message::ProxyConfigRestored(result) => {
            app.busy = false;
            match result {
                Ok(message) => set_status(app, true, message),
                Err(error) => set_status(app, false, error),
            }
            Task::none()
        }
        Message::ClearStatus => {
            app.status = None;
            Task::none()
        }
    }
}

fn subscription(_app: &App) -> Subscription<Message> {
    Subscription::batch([
        time::every(Duration::from_millis(700)).map(|_| Message::Tick),
        iced::keyboard::listen().map(Message::KeyboardEvent),
    ])
}

fn rescan(app: &mut App) -> Task<Message> {
    if let Some(core) = app.core.clone() {
        app.busy = true;
        Task::perform(
            async move {
                let binaries = core.rescan_binaries().await;
                let services = devbox_core::scan_services().await;
                (binaries.map_err(|error| error.to_string()), services)
            },
            Message::BinaryScanCompleted,
        )
    } else {
        Task::none()
    }
}

fn save_binary(app: &mut App) -> Task<Message> {
    let name = app.binary_form.name.trim().to_string();
    let path = app.binary_form.path.trim().to_string();
    if name.is_empty() || path.is_empty() {
        set_status(app, false, "Name and executable path are required".into());
        return Task::none();
    }
    let tested = app
        .binary_form
        .probe
        .as_ref()
        .is_some_and(|(tested_path, result)| tested_path == &path && result.version.is_some());
    if !tested {
        set_status(
            app,
            false,
            "Test this exact executable path before saving".into(),
        );
        return Task::none();
    }
    let category = match app.binary_form.category.as_str() {
        "Tool" => BinaryCategory::Tool,
        "Infrastructure" => BinaryCategory::Infrastructure,
        _ => BinaryCategory::Runtime,
    };
    let arguments = parse_arguments(&app.binary_form.arguments);
    let regex = nonempty(&app.binary_form.version_regex);
    let Some(core) = app.core.clone() else {
        return Task::none();
    };
    app.busy = true;
    Task::perform(
        async move {
            core.save_custom_binary(name, category, path, arguments, regex)
                .await
                .map_err(|error| error.to_string())
        },
        Message::BinarySaved,
    )
}

fn add_project(app: &mut App) -> Task<Message> {
    let name = app.project_form.name.trim().to_string();
    let path = app.project_form.path.trim().to_string();
    if name.is_empty() || path.is_empty() {
        set_status(app, false, "Project name and directory are required".into());
        return Task::none();
    }
    let Some(core) = app.core.clone() else {
        return Task::none();
    };
    let runtime_id = app
        .project_form
        .runtime
        .as_ref()
        .map(|runtime| runtime.id.clone());
    let arguments = parse_arguments(&app.project_form.arguments);
    let environment_variables = app.project_form.environment_variables.clone();
    app.busy = true;
    Task::perform(
        async move {
            core.add_project_with_environment(
                name,
                path,
                runtime_id,
                arguments,
                environment_variables,
            )
            .await
            .map_err(|error| error.to_string())
        },
        Message::ProjectAdded,
    )
}

fn save_project_configuration(app: &mut App) -> Task<Message> {
    let Some(core) = app.core.clone() else {
        return Task::none();
    };
    let Some(form) = app.project_config_form.as_ref() else {
        return Task::none();
    };
    let project_id = form.project_id.clone();
    let runtime_binary_id = form.runtime_binary_id.clone();
    let startup_arguments = parse_arguments(&form.arguments);
    let environment_variables = form.environment_variables.clone();
    app.busy = true;
    Task::perform(
        async move {
            core.update_project_configuration(
                project_id,
                runtime_binary_id,
                startup_arguments,
                environment_variables,
            )
            .await
            .map_err(|error| error.to_string())
        },
        Message::ProjectConfigSaved,
    )
}

fn save_discovery_settings(app: &mut App) -> Task<Message> {
    let Some(core) = app.core.clone() else {
        return Task::none();
    };
    let directories = app.binary_search_directories.clone();
    let automatic_rescan = app.automatic_rescan;
    app.busy = true;
    Task::perform(
        async move {
            core.save_discovery_settings(directories, automatic_rescan)
                .await
                .map_err(|error| error.to_string())
        },
        Message::DiscoverySettingsSaved,
    )
}

fn view(app: &App) -> Element<'_, Message> {
    let palette = colors(app.dark_mode);
    let body = match app.page {
        Page::Dashboard => dashboard(app, palette),
        Page::Projects => projects_page(app, palette),
        Page::Workspaces => workspaces_page(app, palette),
        Page::Binaries => binaries_page(app, palette),
        Page::Processes => processes_page(app, palette),
        Page::Services => services_page(app, palette),
        Page::Networking => networking_page(app, palette),
        Page::Extensions => extensions_page(app, palette),
        Page::Settings => settings_page(app, palette),
    };
    let content = column![toolbar(app, palette), body]
        .spacing(20)
        .padding([24, 28]);
    let main = container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_| container::Style {
            background: Some(Background::Color(palette.background)),
            ..Default::default()
        });
    container(row![sidebar(app, palette), main].spacing(0))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_| container::Style {
            background: Some(Background::Color(palette.background)),
            ..Default::default()
        })
        .into()
}

fn sidebar(app: &App, palette: Colors) -> Element<'_, Message> {
    let groups = [
        ("Overview", vec![(Page::Dashboard, AppIcon::Dashboard)]),
        (
            "Development",
            vec![
                (Page::Projects, AppIcon::Projects),
                (Page::Workspaces, AppIcon::Workspaces),
                (Page::Binaries, AppIcon::Binary),
            ],
        ),
        (
            "Runtime & network",
            vec![
                (Page::Processes, AppIcon::Logs),
                (Page::Services, AppIcon::Services),
                (Page::Networking, AppIcon::Globe),
            ],
        ),
        ("Tools", vec![(Page::Extensions, AppIcon::Extensions)]),
        ("Preferences", vec![(Page::Settings, AppIcon::Settings)]),
    ];
    let mut items = column![
        row![
            container(icon(AppIcon::Runtimes, 21.0, palette.accent))
                .padding(7)
                .style(move |_| container::Style {
                    background: Some(Background::Color(palette.nav_selected)),
                    border: Border {
                        color: palette.nav_border,
                        width: 1.0,
                        radius: 10.0.into(),
                    },
                    ..Default::default()
                }),
            column![
                text("Devbox").size(18).color(palette.text),
                text("Local development").size(10).color(palette.secondary),
            ]
            .spacing(1),
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center),
    ]
    .spacing(18);
    for (section, pages) in groups {
        let mut links = column![].spacing(3);
        for (page, app_icon) in pages {
            let selected = page == app.page;
            let control = button(
                row![
                    container(iced::widget::Space::new().height(Length::Fixed(17.0)))
                        .width(Length::Fixed(3.0))
                        .style(move |_| container::Style {
                            background: Some(Background::Color(if selected {
                                palette.accent
                            } else {
                                Color::TRANSPARENT
                            })),
                            border: Border {
                                radius: 3.0.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                    icon(
                        app_icon,
                        17.0,
                        if selected {
                            palette.accent
                        } else {
                            palette.secondary
                        }
                    ),
                    text(page.title()).size(13),
                ]
                .spacing(7)
                .align_y(iced::Alignment::Center),
            )
            .width(Length::Fill)
            .padding([7, 5])
            .style(move |_, status| nav_button_style(palette, selected, status));
            links = links.push(control.on_press(Message::Navigate(page)));
        }
        items =
            items.push(column![text(section).size(11).color(palette.secondary), links,].spacing(6));
    }
    let bottom = column![
        container(iced::widget::Space::new().height(Length::Fixed(1.0)))
            .width(Length::Fill)
            .style(move |_| container::Style {
                background: Some(Background::Color(palette.separator)),
                ..Default::default()
            }),
        text(if let Some(error) = &app.startup_error {
            error.as_str()
        } else {
            "Linux host · native processes"
        })
        .size(10)
        .color(if app.startup_error.is_some() {
            palette.warning
        } else {
            palette.secondary
        }),
        text("Devbox 0.1 · Early milestone")
            .size(9)
            .color(palette.secondary),
        text("Ctrl/⌘ + 1–9 · Tab through fields")
            .size(9)
            .color(palette.secondary),
    ]
    .spacing(9);
    container(
        column![
            items,
            iced::widget::Space::new().height(Length::Fill),
            bottom
        ]
        .spacing(12)
        .height(Length::Fill),
    )
    .width(Length::Fixed(224.0))
    .height(Length::Fill)
    .padding([22, 15])
    .style(move |_| container::Style {
        background: Some(Background::Color(palette.sidebar)),
        border: Border {
            color: palette.separator,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    })
    .into()
}

fn toolbar(app: &App, palette: Colors) -> Element<'_, Message> {
    let title = column![
        text(app.page.title()).size(25).color(palette.text),
        text(app.page.subtitle()).size(13).color(palette.secondary),
    ]
    .spacing(5);
    let mut actions = row![
        button(
            row![
                icon(AppIcon::Refresh, 16.0, palette.secondary),
                text("Rescan").size(13)
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center)
        )
        .padding([9, 12])
        .style(secondary_button_style(palette))
        .on_press(Message::Refresh),
    ]
    .spacing(10)
    .align_y(iced::Alignment::Center);
    if app.page == Page::Binaries {
        actions = actions.push(
            button(
                row![
                    icon(AppIcon::Plus, 16.0, Color::WHITE),
                    text("Add binary").size(13).color(Color::WHITE)
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            )
            .padding([9, 13])
            .style(primary_button_style(palette))
            .on_press(Message::ToggleBinaryForm),
        );
    }
    if app.page == Page::Projects {
        actions = actions.push(
            button(
                row![
                    icon(AppIcon::Plus, 16.0, Color::WHITE),
                    text("Add project").size(13).color(Color::WHITE)
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            )
            .padding([9, 13])
            .style(primary_button_style(palette))
            .on_press(Message::ToggleProjectForm),
        );
    }
    let status = if let Some((success, message)) = &app.status {
        Some(
            row![
                icon(
                    if *success {
                        AppIcon::Check
                    } else {
                        AppIcon::Warning
                    },
                    15.0,
                    if *success {
                        palette.success
                    } else {
                        palette.warning
                    }
                ),
                text(message).size(12).color(palette.secondary),
                button(
                    row![
                        icon(AppIcon::Error, 12.0, palette.secondary),
                        text("Dismiss").size(10).color(palette.secondary)
                    ]
                    .spacing(5)
                    .align_y(iced::Alignment::Center)
                )
                .padding([1, 5])
                .style(quiet_button_style(palette))
                .on_press(Message::ClearStatus),
            ]
            .spacing(7)
            .align_y(iced::Alignment::Center),
        )
    } else if app.busy {
        Some(row![text("Working…").size(12).color(palette.secondary)])
    } else {
        None
    };
    let mut right = column![actions].spacing(12).align_x(iced::Alignment::End);
    if let Some(status) = status {
        right = right.push(status);
    }
    let heading = row![
        container(icon(app.page.icon(), 20.0, palette.accent))
            .padding(9)
            .style(move |_| container::Style {
                background: Some(Background::Color(palette.nav_selected)),
                border: Border {
                    color: palette.nav_border,
                    width: 1.0,
                    radius: 11.0.into(),
                },
                ..Default::default()
            }),
        title,
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center);
    row![
        heading,
        iced::widget::Space::new().width(Length::Fill),
        right
    ]
    .align_y(iced::Alignment::Center)
    .into()
}

fn scroll_area<'a>(
    content: impl Into<Element<'a, Message>>,
    height: Length,
) -> Element<'a, Message> {
    scrollable(container(content).width(Length::Fill))
        .direction(iced::widget::scrollable::Direction::Vertical(
            iced::widget::scrollable::Scrollbar::new().spacing(8.0),
        ))
        .width(Length::Fill)
        .height(height)
        .into()
}

fn dashboard(app: &App, palette: Colors) -> Element<'_, Message> {
    let running = app
        .processes
        .iter()
        .filter(|process| {
            matches!(
                process.status,
                ProcessStatus::Running | ProcessStatus::Stopping
            )
        })
        .count();
    let stats = card(
        row![
            stat_summary(
                AppIcon::Binary,
                "Detected binaries",
                app.binaries.len().to_string(),
                "Available in PATH or registered",
                palette
            ),
            metric_divider(palette),
            stat_summary(
                AppIcon::Projects,
                "Projects",
                app.projects.len().to_string(),
                "Existing folders registered",
                palette
            ),
            metric_divider(palette),
            stat_summary(
                AppIcon::Activity,
                "Active processes",
                running.to_string(),
                "Started by this Devbox session",
                palette
            ),
        ]
        .spacing(13)
        .align_y(iced::Alignment::Center),
        palette,
    );
    let quick = card(
        column![
            text("Get started").size(17).color(palette.text),
            text("Scan your host for familiar development tools, then register a project and choose the executable it should use.")
                .size(13).color(palette.secondary),
            row![
                action_button(AppIcon::Scan, "Scan runtimes", Message::Refresh, palette),
                action_button(AppIcon::Projects, "Open projects", Message::Navigate(Page::Projects), palette),
                action_button(AppIcon::Binary, "Manage binaries", Message::Navigate(Page::Binaries), palette),
            ].spacing(10),
        ].spacing(13),
        palette,
    );
    let recent = if app.processes.is_empty() {
        card(
            column![
                text("Recent activity").size(17).color(palette.text),
                text("No project processes have been started yet.")
                    .size(13)
                    .color(palette.secondary),
            ]
            .spacing(10),
            palette,
        )
    } else {
        card(
            column![
                text("Recent activity").size(17).color(palette.text),
                column(app.processes.iter().take(4).map(|process| {
                    row![
                        status_dot(&process.status, palette),
                        text(&process.project_name).size(13).color(palette.text),
                        iced::widget::Space::new().width(Length::Fill),
                        text(format!(
                            "PID {} · {}",
                            process.pid,
                            status_label(&process.status)
                        ))
                        .size(12)
                        .color(palette.secondary),
                    ]
                    .spacing(10)
                    .align_y(iced::Alignment::Center)
                    .into()
                }))
                .spacing(10),
            ]
            .spacing(14),
            palette,
        )
    };
    scroll_area(column![stats, quick, recent].spacing(16), Length::Fill)
}

fn stat_summary(
    icon_name: AppIcon,
    title: &str,
    value: String,
    caption: &str,
    palette: Colors,
) -> Element<'static, Message> {
    column![
        row![
            icon(icon_name, 17.0, palette.accent),
            text(title.to_string()).size(12).color(palette.secondary)
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
        text(value).size(27).color(palette.text),
        text(caption.to_string()).size(10).color(palette.secondary),
    ]
    .spacing(8)
    .into()
}

fn metric_divider(palette: Colors) -> Element<'static, Message> {
    container(iced::widget::Space::new().height(Length::Fixed(52.0)))
        .width(Length::Fixed(1.0))
        .style(move |_| container::Style {
            background: Some(Background::Color(palette.separator)),
            ..Default::default()
        })
        .into()
}

fn projects_page(app: &App, palette: Colors) -> Element<'_, Message> {
    let mut content = column![].spacing(14);
    if app.show_project_form {
        let choices = runtime_choices(&app.binaries);
        let mut environment_rows = column![].spacing(7);
        if app.project_form.environment_variables.is_empty() {
            environment_rows = environment_rows.push(
                text("No project environment variables")
                    .size(11)
                    .color(palette.secondary),
            );
        }
        for variable in &app.project_form.environment_variables {
            let displayed_value = if variable.secret {
                "••••••••".to_string()
            } else {
                variable.value.clone()
            };
            let variable_key = variable.key.clone();
            environment_rows = environment_rows.push(
                row![
                    text(format!("{} = {}", variable.key, displayed_value))
                        .size(11)
                        .color(palette.text),
                    text(if variable.secret {
                        "Sensitive"
                    } else {
                        "Visible"
                    })
                    .size(10)
                    .color(palette.secondary),
                    iced::widget::Space::new().width(Length::Fill),
                    button(text("Remove").size(10).color(palette.error))
                        .padding([5, 8])
                        .style(secondary_button_style(palette))
                        .on_press(Message::RemoveEnvironmentVariable(variable_key)),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            );
        }
        let form = column![
            text("Register an existing project").size(17).color(palette.text),
            text("Devbox keeps source files where they are. Startup arguments are split on spaces and passed directly to the chosen executable.")
                .size(12).color(palette.secondary),
            form_group(
                column![
                    form_group_heading(
                        AppIcon::Projects,
                        "Project launch",
                        "Choose the existing folder and runtime for this project.",
                        palette,
                    ),
                    row![
                        form_field(
                            "Project name",
                            text_input("", &app.project_form.name)
                                .on_input(Message::ProjectNameChanged)
                                .padding([9, 11])
                                .style(text_input_style(palette))
                                .width(Length::Fill),
                            Length::FillPortion(1),
                            palette,
                        ),
                        form_field(
                            "Project folder",
                            row![
                                text_input("Choose an existing folder", &app.project_form.path)
                                    .on_input(Message::ProjectPathChanged)
                                    .padding([9, 11])
                                    .style(text_input_style(palette))
                                    .width(Length::Fill),
                                button(
                                    row![
                                        icon(AppIcon::OpenFolder, 14.0, palette.secondary),
                                        text("Browse").size(12)
                                    ]
                                    .spacing(7)
                                    .align_y(iced::Alignment::Center)
                                )
                                .padding([12, 11])
                                .style(secondary_button_style(palette))
                                .on_press(Message::PickProjectFolder),
                            ]
                            .spacing(8)
                            .align_y(iced::Alignment::Center),
                            Length::FillPortion(2),
                            palette,
                        ),
                    ]
                    .spacing(10),
                    form_field(
                        "Runtime",
                        pick_list(choices, app.project_form.runtime.clone(), Message::ProjectRuntimeSelected)
                            .placeholder("Optional · use the host default")
                            .padding([9, 11])
                            .style(pick_list_style(palette))
                            .menu_style(pick_menu_style(palette))
                            .width(Length::Fill),
                        Length::Fill,
                        palette,
                    ),
                    form_field(
                        "Startup arguments",
                        text_input("Split on spaces and passed directly to the runtime", &app.project_form.arguments)
                            .on_input(Message::ProjectArgumentsChanged)
                            .padding([9, 11])
                            .style(text_input_style(palette))
                            .width(Length::Fill),
                        Length::Fill,
                        palette,
                    ),
                ]
                .spacing(10),
                palette,
            ),
            form_group(
                column![
                    form_group_heading(
                        AppIcon::Settings,
                        "Environment variables",
                        "Only this project process receives these values. Sensitive values are hidden from captured logs.",
                        palette,
                    ),
                    row![
                        form_field(
                            "Name",
                            text_input("e.g. APP_ENV", &app.project_form.environment_key)
                                .on_input(Message::EnvironmentKeyChanged)
                                .padding([9, 11])
                                .style(text_input_style(palette))
                                .width(Length::Fill),
                            Length::FillPortion(1),
                            palette,
                        ),
                        form_field(
                            "Value",
                            text_input("Environment value", &app.project_form.environment_value)
                                .on_input(Message::EnvironmentValueChanged)
                                .secure(app.project_form.environment_secret)
                                .padding([9, 11])
                                .style(text_input_style(palette))
                                .width(Length::Fill),
                            Length::FillPortion(2),
                            palette,
                        ),
                    ]
                    .spacing(9),
                    row![
                        checkbox(app.project_form.environment_secret)
                            .label("Sensitive value")
                            .style(checkbox_style(palette))
                            .on_toggle(Message::EnvironmentSecretChanged),
                        iced::widget::Space::new().width(Length::Fill),
                        button(
                            row![
                                icon(AppIcon::Plus, 14.0, palette.accent),
                                text("Add variable").size(12).color(palette.accent)
                            ]
                            .spacing(6)
                            .align_y(iced::Alignment::Center)
                        )
                        .padding([12, 11])
                        .style(secondary_button_style(palette))
                        .on_press(Message::AddEnvironmentVariable),
                    ]
                    .spacing(12)
                    .align_y(iced::Alignment::Center),
                    environment_rows,
                ]
                .spacing(10),
                palette,
            ),
            row![
                button(text("Cancel").size(13)).padding([12, 13]).style(secondary_button_style(palette)).on_press(Message::ToggleProjectForm),
                button(text("Register project").size(13).color(Color::WHITE)).padding([12, 13]).style(primary_button_style(palette)).on_press(Message::AddProject),
            ].spacing(9),
        ].spacing(12);
        content = content.push(card(form, palette));
    }
    if app.projects.is_empty() && !app.show_project_form {
        content = content.push(empty_state(
            AppIcon::Projects,
            "No projects registered",
            "Add a project folder to choose its runtime and launch it as a native process.",
            palette,
        ));
    }
    let choices = runtime_choices(&app.binaries);
    for project in &app.projects {
        let active = app.processes.iter().find(|process| {
            process.project_id == project.id
                && matches!(
                    process.status,
                    ProcessStatus::Running | ProcessStatus::Stopping
                )
        });
        let start_stop = if active.is_some() {
            button(
                row![
                    icon(AppIcon::Stop, 14.0, palette.error),
                    text("Stop").size(12).color(palette.error)
                ]
                .spacing(7)
                .align_y(iced::Alignment::Center),
            )
            .padding([8, 11])
            .style(secondary_button_style(palette))
            .on_press(Message::StopProject(project.id.clone()))
        } else {
            button(
                row![
                    icon(AppIcon::Start, 14.0, palette.accent),
                    text("Start").size(12).color(palette.accent)
                ]
                .spacing(7)
                .align_y(iced::Alignment::Center),
            )
            .padding([8, 11])
            .style(secondary_button_style(palette))
            .on_press(Message::StartProject(project.id.clone()))
        };
        let runtime_label = selected_runtime_label(project, &app.binaries);
        let config_is_open = app
            .project_config_form
            .as_ref()
            .is_some_and(|form| form.project_id == project.id);
        let configure = button(
            row![
                icon(AppIcon::Settings, 14.0, palette.secondary),
                text(if config_is_open {
                    "Close config"
                } else {
                    "Configure"
                })
                .size(12)
                .color(palette.text)
            ]
            .spacing(7)
            .align_y(iced::Alignment::Center),
        )
        .padding([8, 11])
        .style(secondary_button_style(palette))
        .on_press(Message::OpenProjectConfig(project.id.clone()));
        let config_editor = app
            .project_config_form
            .as_ref()
            .filter(|form| form.project_id == project.id)
            .map(|form| project_config_editor(form, choices.clone(), palette, active.is_some()));
        let mut item_content = column![
            row![
                icon(AppIcon::Projects, 20.0, palette.accent),
                column![
                    text(&project.name).size(15).color(palette.text),
                    text(&project.path).size(11).color(palette.secondary),
                    text(if project.detected_frameworks.is_empty() {
                        "Framework not detected".to_string()
                    } else {
                        format!("Detected: {}", project.detected_frameworks.join(", "))
                    })
                    .size(10)
                    .color(palette.secondary),
                ]
                .spacing(5),
                iced::widget::Space::new().width(Length::Fill),
                status_dot(
                    active.map(|p| &p.status).unwrap_or(&ProcessStatus::Exited),
                    palette
                ),
                text(active.map(|p| status_label(&p.status)).unwrap_or("Stopped"))
                    .size(12)
                    .color(palette.secondary),
            ]
            .spacing(11)
            .align_y(iced::Alignment::Center),
            row![
                text(format!("Runtime: {runtime_label}"))
                    .size(12)
                    .color(palette.secondary),
                iced::widget::Space::new().width(Length::Fill),
                configure,
                start_stop,
            ]
            .spacing(9)
            .align_y(iced::Alignment::Center),
            text(if project.startup_arguments.is_empty() {
                "Arguments: none".to_string()
            } else {
                format!("Arguments: {}", project.startup_arguments.join(" "))
            })
            .size(11)
            .color(palette.secondary),
        ]
        .spacing(12);
        if let Some(config_editor) = config_editor {
            item_content = item_content.push(config_editor);
        }
        let item = card(item_content, palette);
        content = content.push(item);
    }
    scroll_area(content, Length::Fill)
}

fn project_config_editor<'a>(
    form: &'a ProjectConfigForm,
    choices: Vec<RuntimeChoice>,
    palette: Colors,
    is_running: bool,
) -> Element<'a, Message> {
    let runtime_placeholder = if form.runtime_binary_id.is_some() && form.runtime.is_none() {
        "Registered runtime unavailable"
    } else {
        "Use host default"
    };
    let runtime_picker = pick_list(
        choices,
        form.runtime.clone(),
        Message::ProjectConfigRuntimeSelected,
    )
    .placeholder(runtime_placeholder)
    .padding([9, 11])
    .style(pick_list_style(palette))
    .menu_style(pick_menu_style(palette))
    .width(Length::Fill);
    let use_host_default: Element<'a, Message> = if form.runtime_binary_id.is_some() {
        button(text("Host default").size(11).color(palette.secondary))
            .padding([9, 10])
            .style(secondary_button_style(palette))
            .on_press(Message::ProjectConfigUseHostDefault)
            .into()
    } else {
        iced::widget::Space::new().width(Length::Shrink).into()
    };
    let mut environment_rows = column![].spacing(7);
    if form.environment_variables.is_empty() {
        environment_rows = environment_rows.push(
            text("No environment variables configured")
                .size(11)
                .color(palette.secondary),
        );
    }
    for variable in &form.environment_variables {
        let displayed_value = if variable.secret {
            "••••••••".to_string()
        } else {
            variable.value.clone()
        };
        let key = variable.key.clone();
        environment_rows = environment_rows.push(
            row![
                text(format!("{} = {}", variable.key, displayed_value))
                    .size(11)
                    .color(palette.text),
                text(if variable.secret {
                    "Sensitive"
                } else {
                    "Visible"
                })
                .size(10)
                .color(palette.secondary),
                iced::widget::Space::new().width(Length::Fill),
                button(text("Remove").size(10).color(palette.error))
                    .padding([5, 8])
                    .style(secondary_button_style(palette))
                    .on_press(Message::RemoveProjectConfigEnvironmentVariable(key)),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        );
    }
    let runtime_and_arguments = row![
        form_field(
            "Runtime",
            row![runtime_picker, use_host_default]
                .spacing(7)
                .align_y(iced::Alignment::Center),
            Length::FillPortion(1),
            palette,
        ),
        form_field(
            "Startup arguments",
            text_input("e.g. dev --port 3000", &form.arguments)
                .on_input(Message::ProjectConfigArgumentsChanged)
                .padding([9, 11])
                .style(text_input_style(palette))
                .width(Length::Fill),
            Length::FillPortion(2),
            palette,
        ),
    ]
    .spacing(10);
    let environment_add = row![
        form_field(
            "Variable name",
            text_input("e.g. APP_ENV", &form.environment_key)
                .on_input(Message::ProjectConfigEnvironmentKeyChanged)
                .padding([9, 11])
                .style(text_input_style(palette))
                .width(Length::Fill),
            Length::FillPortion(1),
            palette,
        ),
        form_field(
            "Value",
            text_input("Environment value", &form.environment_value)
                .on_input(Message::ProjectConfigEnvironmentValueChanged)
                .secure(form.environment_secret)
                .padding([9, 11])
                .style(text_input_style(palette))
                .width(Length::Fill),
            Length::FillPortion(2),
            palette,
        ),
        checkbox(form.environment_secret)
            .label("Sensitive")
            .on_toggle(Message::ProjectConfigEnvironmentSecretChanged)
            .style(checkbox_style(palette)),
        button(text("Add variable").size(11).color(palette.accent))
            .padding([9, 11])
            .style(secondary_button_style(palette))
            .on_press(Message::AddProjectConfigEnvironmentVariable),
    ]
    .spacing(9)
    .align_y(iced::Alignment::End);
    let next_start_note = if is_running {
        "Changes apply the next time this project starts. The running process continues unchanged."
    } else {
        "These settings will be used the next time you start this project."
    };
    form_group(
        column![
            runtime_and_arguments,
            text("Environment variables").size(13).color(palette.text),
            environment_rows,
            environment_add,
            row![
                text(next_start_note)
                    .size(11)
                    .color(palette.secondary)
                    .width(Length::Fill),
                button(text("Cancel").size(12))
                    .padding([9, 12])
                    .style(secondary_button_style(palette))
                    .on_press(Message::CloseProjectConfig),
                button(text("Save configuration").size(12).color(Color::WHITE))
                    .padding([9, 12])
                    .style(primary_button_style(palette))
                    .on_press(Message::SaveProjectConfig),
            ]
            .spacing(9)
            .align_y(iced::Alignment::Center),
        ]
        .spacing(11),
        palette,
    )
}

fn binaries_page(app: &App, palette: Colors) -> Element<'_, Message> {
    let mut content = column![
        row![
            text(format!("{} registered binaries", app.binaries.len()))
                .size(13)
                .color(palette.secondary),
            iced::widget::Space::new().width(Length::Fill),
            button(
                row![
                    icon(AppIcon::Refresh, 14.0, palette.secondary),
                    text("Rescan all").size(12)
                ]
                .spacing(7)
                .align_y(iced::Alignment::Center)
            )
            .padding([8, 10])
            .style(secondary_button_style(palette))
            .on_press(Message::Refresh),
        ]
        .align_y(iced::Alignment::Center),
    ]
    .spacing(12);
    if app.show_binary_form {
        let category_options = vec![
            "Runtime".to_string(),
            "Tool".to_string(),
            "Infrastructure".to_string(),
        ];
        let current_probe = app
            .binary_form
            .probe
            .as_ref()
            .filter(|(path, _)| path == &app.binary_form.path);
        let probe_line: Element<'_, Message> = if let Some((_, result)) = current_probe {
            if let Some(version) = &result.version {
                row![
                    icon(AppIcon::Check, 15.0, palette.success),
                    text(format!("Available · {version}"))
                        .size(12)
                        .color(palette.success)
                ]
                .spacing(7)
                .into()
            } else {
                row![
                    icon(AppIcon::Warning, 15.0, palette.warning),
                    text(
                        result
                            .error
                            .clone()
                            .unwrap_or_else(|| "Not available".into())
                    )
                    .size(12)
                    .color(palette.warning)
                ]
                .spacing(7)
                .into()
            }
        } else {
            row![
                text("Test the executable to confirm its version before saving.")
                    .size(12)
                    .color(palette.secondary)
            ]
            .into()
        };
        let form = column![
            text("Add custom binary").size(17).color(palette.text),
            form_group(
                column![
                    form_group_heading(
                        AppIcon::Binary,
                        "Executable",
                        "Identify the binary and where Devbox can find it.",
                        palette,
                    ),
                    row![
                        form_field(
                            "Display name",
                            text_input("e.g. PHP 8.3 custom", &app.binary_form.name)
                                .on_input(Message::BinaryNameChanged)
                                .padding([9, 11])
                                .style(text_input_style(palette))
                                .width(Length::Fill),
                            Length::FillPortion(2),
                            palette,
                        ),
                        form_field(
                            "Category",
                            pick_list(
                                category_options,
                                Some(app.binary_form.category.clone()),
                                Message::BinaryCategoryChanged
                            )
                            .padding([9, 11])
                            .style(pick_list_style(palette))
                            .menu_style(pick_menu_style(palette))
                            .width(Length::Fill),
                            Length::FillPortion(1),
                            palette,
                        ),
                    ]
                    .spacing(10),
                    form_field(
                        "Executable path",
                        row![
                            text_input("e.g. /opt/php83/bin/php", &app.binary_form.path)
                                .on_input(Message::BinaryPathChanged)
                                .padding([9, 11])
                                .style(text_input_style(palette))
                                .width(Length::Fill),
                            button(
                                row![
                                    icon(AppIcon::OpenFolder, 14.0, palette.secondary),
                                    text("Browse").size(12)
                                ]
                                .spacing(7)
                                .align_y(iced::Alignment::Center)
                            )
                            .padding([12, 11])
                            .style(secondary_button_style(palette))
                            .on_press(Message::PickBinaryPath),
                        ]
                        .spacing(8)
                        .align_y(iced::Alignment::Center),
                        Length::Fill,
                        palette,
                    ),
                ]
                .spacing(10),
                palette,
            ),
            form_group(
                column![
                    form_group_heading(
                        AppIcon::Runtimes,
                        "Version detection",
                        "Set how Devbox checks this executable before saving it.",
                        palette,
                    ),
                    row![
                        form_field(
                            "Version arguments",
                            text_input("e.g. --version", &app.binary_form.arguments)
                                .on_input(Message::BinaryArgumentsChanged)
                                .padding([9, 11])
                                .style(text_input_style(palette))
                                .width(Length::Fill),
                            Length::FillPortion(1),
                            palette,
                        ),
                        form_field(
                            "Optional version pattern",
                            text_input(
                                "Regex used to extract the version",
                                &app.binary_form.version_regex
                            )
                            .on_input(Message::BinaryRegexChanged)
                            .padding([9, 11])
                            .style(text_input_style(palette))
                            .width(Length::Fill),
                            Length::FillPortion(2),
                            palette,
                        ),
                    ]
                    .spacing(10),
                    row![
                        button(
                            row![
                                icon(AppIcon::Check, 14.0, palette.accent),
                                text("Test binary").size(12).color(palette.accent)
                            ]
                            .spacing(7)
                            .align_y(iced::Alignment::Center)
                        )
                        .padding([12, 13])
                        .style(secondary_button_style(palette))
                        .on_press(Message::TestBinary),
                        probe_line,
                    ]
                    .spacing(10)
                    .align_y(iced::Alignment::Center),
                ]
                .spacing(10),
                palette,
            ),
            row![
                iced::widget::Space::new().width(Length::Fill),
                button(text("Cancel").size(12))
                    .padding([12, 13])
                    .style(secondary_button_style(palette))
                    .on_press(Message::ToggleBinaryForm),
                button(text("Save registration").size(12).color(Color::WHITE))
                    .padding([12, 13])
                    .style(primary_button_style(palette))
                    .on_press(Message::SaveBinary),
            ]
            .spacing(9),
        ]
        .spacing(11);
        content = content.push(card(form, palette));
    }
    if app.binaries.is_empty() {
        content = content.push(empty_state(
            AppIcon::Binary,
            "No binaries found yet",
            "Rescan PATH or register a custom executable path.",
            palette,
        ));
    }
    for binary in &app.binaries {
        let symbol = match &binary.category {
            BinaryCategory::Runtime => AppIcon::Runtimes,
            BinaryCategory::Tool => AppIcon::Extensions,
            BinaryCategory::Infrastructure => AppIcon::Services,
        };
        let version = binary.version().unwrap_or("Version not detected");
        let remove_id = binary.id.clone();
        let is_php = matches!(&binary.category, BinaryCategory::Runtime)
            && binary.display_name.to_ascii_lowercase().contains("php");
        let php_action: Element<'_, Message> = if is_php {
            if app.php_inspections.contains(&binary.id) {
                text("Inspecting PHP extensions…")
                    .size(10)
                    .color(palette.secondary)
                    .into()
            } else {
                button(
                    text("Inspect PHP extensions")
                        .size(11)
                        .color(palette.accent),
                )
                .padding([6, 8])
                .style(secondary_button_style(palette))
                .on_press(Message::InspectPhpExtensions(binary.id.clone()))
                .into()
            }
        } else {
            iced::widget::Space::new().height(Length::Fixed(0.0)).into()
        };
        let php_report: Element<'_, Message> = match app.php_reports.get(&binary.id) {
            Some(Ok(report)) => column![
                text(format!(
                    "{} loaded extensions: {}",
                    report.loaded_extensions.len(),
                    report.loaded_extensions.join(", ")
                ))
                .size(10)
                .color(palette.secondary),
                text(format!(
                    "Loaded php.ini: {}",
                    report.loaded_configuration.as_deref().unwrap_or("none")
                ))
                .size(10)
                .color(palette.secondary),
                text(format!(
                    "Additional INI files: {}",
                    if report.additional_ini_files.is_empty() {
                        "none".to_string()
                    } else {
                        report.additional_ini_files.join(", ")
                    }
                ))
                .size(10)
                .color(palette.secondary),
            ]
            .spacing(3)
            .into(),
            Some(Err(error)) => text(format!("PHP inspection failed: {error}"))
                .size(10)
                .color(palette.warning)
                .into(),
            None => iced::widget::Space::new().height(Length::Fixed(0.0)).into(),
        };
        let probe_detail: Element<'_, Message> = if let Some(error) = &binary.probe_error {
            row![
                icon(AppIcon::Warning, 13.0, palette.warning),
                text(format!("Version probe failed: {error}"))
                    .size(10)
                    .color(palette.warning)
            ]
            .spacing(6)
            .into()
        } else {
            iced::widget::Space::new().height(Length::Fixed(0.0)).into()
        };
        content = content.push(card(
            row![
                icon(symbol, 21.0, palette.accent),
                column![
                    row![
                        text(&binary.display_name).size(14).color(palette.text),
                        container(
                            text(binary.category.as_str())
                                .size(10)
                                .color(palette.accent),
                        )
                        .padding([3, 7])
                        .style(move |_| container::Style {
                            background: Some(Background::Color(palette.nav_selected)),
                            border: Border {
                                radius: 6.0.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                    ]
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
                    text(&binary.executable_path)
                        .size(11)
                        .color(palette.secondary),
                    text(format!(
                        "{} · {}",
                        binary.discovery_source, binary.trust_status
                    ))
                    .size(10)
                    .color(palette.secondary),
                    probe_detail,
                    php_action,
                    php_report,
                ]
                .spacing(5),
                iced::widget::Space::new().width(Length::Fill),
                column![
                    text(version).size(13).color(palette.text),
                    text(if binary.enabled {
                        "Enabled"
                    } else {
                        "Disabled"
                    })
                    .size(10)
                    .color(palette.secondary)
                ]
                .align_x(iced::Alignment::End)
                .spacing(5),
                button(text("Remove").size(11).color(palette.error))
                    .padding([7, 9])
                    .style(secondary_button_style(palette))
                    .on_press(Message::RemoveBinary(remove_id)),
            ]
            .spacing(12)
            .align_y(iced::Alignment::Center),
            palette,
        ));
    }
    scroll_area(content, Length::Fill)
}

fn processes_page(app: &App, palette: Colors) -> Element<'_, Message> {
    let mut content = column![
        row![
            form_field(
                "Search logs",
                text_input("Search process output", &app.log_query)
                    .on_input(Message::LogQueryChanged)
                    .padding([9, 11])
                    .style(text_input_style(palette))
                    .width(Length::Fill),
                Length::FillPortion(2),
                palette,
            ),
            form_field(
                "Output stream",
                pick_list(
                    LogStreamChoice::ALL.to_vec(),
                    Some(app.log_stream),
                    Message::LogStreamChanged,
                )
                .padding([9, 11])
                .style(pick_list_style(palette))
                .menu_style(pick_menu_style(palette))
                .width(Length::Fill),
                Length::FillPortion(1),
                palette,
            ),
        ]
        .spacing(10)
        .align_y(iced::Alignment::End)
    ]
    .spacing(13);
    if app.processes.is_empty() {
        content = content.push(empty_state(
            AppIcon::Logs,
            "No process sessions",
            "Start a registered project to see live stdout and stderr here.",
            palette,
        ));
    }
    for process in &app.processes {
        let is_running = matches!(
            process.status,
            ProcessStatus::Running | ProcessStatus::Stopping
        );
        let controls: Element<'_, Message> = if is_running {
            row![
                button(
                    row![
                        icon(AppIcon::Restart, 14.0, palette.accent),
                        text("Restart").size(12).color(palette.accent)
                    ]
                    .spacing(7)
                    .align_y(iced::Alignment::Center),
                )
                .padding([8, 10])
                .style(secondary_button_style(palette))
                .on_press(Message::RestartProject(process.project_id.clone())),
                button(
                    row![
                        icon(AppIcon::Stop, 14.0, palette.error),
                        text("Stop safely").size(12).color(palette.error)
                    ]
                    .spacing(7)
                    .align_y(iced::Alignment::Center),
                )
                .padding([8, 10])
                .style(secondary_button_style(palette))
                .on_press(Message::StopProject(process.project_id.clone())),
            ]
            .spacing(7)
            .into()
        } else {
            button(
                row![
                    icon(AppIcon::Restart, 14.0, palette.accent),
                    text("Start again").size(12).color(palette.accent)
                ]
                .spacing(7)
                .align_y(iced::Alignment::Center),
            )
            .padding([8, 10])
            .style(secondary_button_style(palette))
            .on_press(Message::StartProject(process.project_id.clone()))
            .into()
        };
        let query = app.log_query.to_lowercase();
        let visible_logs = process
            .logs
            .iter()
            .filter(|line| app.log_stream.matches(line))
            .filter(|line| query.is_empty() || line.to_lowercase().contains(&query))
            .collect::<Vec<_>>();
        let log_content = if visible_logs.is_empty() {
            column![
                text(if process.logs.is_empty() {
                    "Waiting for process output…"
                } else {
                    "No log lines match these filters."
                })
                .size(12)
                .color(palette.secondary)
            ]
        } else {
            column(
                visible_logs
                    .iter()
                    .map(|line| text(*line).size(11).color(palette.log_text).into()),
            )
            .spacing(4)
        };
        content = content.push(card(
            column![
                row![
                    status_dot(&process.status, palette),
                    column![
                        text(&process.project_name).size(15).color(palette.text),
                        text(format!(
                            "PID {} · started {}",
                            process.pid,
                            format_time(process.started_at)
                        ))
                        .size(11)
                        .color(palette.secondary),
                        text(process_runtime_summary(
                            process,
                            is_running,
                            visible_logs.len(),
                        ))
                        .size(10)
                        .color(palette.secondary)
                    ]
                    .spacing(5),
                    iced::widget::Space::new().width(Length::Fill),
                    text(status_label(&process.status))
                        .size(12)
                        .color(process_color(&process.status, palette)),
                    controls,
                ]
                .spacing(10)
                .align_y(iced::Alignment::Center),
                if let Some(detail) = &process.error_detail {
                    text(format!("Process detail: {detail}"))
                        .size(11)
                        .color(palette.error)
                } else if let Some(code) = process.exit_code {
                    text(format!("Exit code: {code}"))
                        .size(11)
                        .color(palette.secondary)
                } else {
                    text("").size(1)
                },
                container(scroll_area(log_content, Length::Fixed(170.0)))
                    .padding(12)
                    .width(Length::Fill)
                    .style(move |_| container::Style {
                        background: Some(Background::Color(palette.log_background)),
                        border: Border {
                            color: palette.separator,
                            width: 1.0,
                            radius: 9.0.into()
                        },
                        ..Default::default()
                    }),
            ]
            .spacing(12),
            palette,
        ));
    }
    scroll_area(content, Length::Fill)
}

fn settings_page(app: &App, palette: Colors) -> Element<'_, Message> {
    let mut directories = column![].spacing(8);
    if app.binary_search_directories.is_empty() {
        directories = directories.push(
            text("No additional search directories")
                .size(12)
                .color(palette.secondary),
        );
    }
    for directory in &app.binary_search_directories {
        directories = directories.push(
            row![
                icon(AppIcon::Binary, 15.0, palette.secondary),
                text(directory).size(12).color(palette.text),
                iced::widget::Space::new().width(Length::Fill),
                button(text("Remove").size(11).color(palette.error))
                    .padding([6, 9])
                    .style(secondary_button_style(palette))
                    .on_press(Message::RemoveDiscoveryDirectory(directory.clone())),
            ]
            .spacing(9)
            .align_y(iced::Alignment::Center),
        );
    }

    let content = column![
        card(
            column![
                text("Binary discovery").size(17).color(palette.text),
                text("Devbox checks these directories directly for supported runtime and tool executables. It does not scan their subdirectories.")
                    .size(12)
                    .color(palette.secondary),
                checkbox(app.automatic_rescan)
                    .label("Rescan binaries when Devbox starts")
                    .style(checkbox_style(palette))
                    .on_toggle(Message::AutomaticRescanChanged),
                row![
                    form_field(
                        "Additional binary directory",
                        row![
                            text_input("Path to a directory", &app.directory_entry)
                                .on_input(Message::DiscoveryDirectoryInputChanged)
                                .padding([9, 11])
                                .style(text_input_style(palette))
                                .width(Length::Fill),
                            button(
                                row![
                                    icon(AppIcon::OpenFolder, 14.0, palette.secondary),
                                    text("Browse").size(12)
                                ]
                                .spacing(7)
                                .align_y(iced::Alignment::Center)
                            )
                            .padding([12, 11])
                            .style(secondary_button_style(palette))
                            .on_press(Message::PickDiscoveryDirectory),
                        ]
                        .spacing(8)
                        .align_y(iced::Alignment::Center),
                        Length::Fill,
                        palette,
                    ),
                    button(
                        row![
                            icon(AppIcon::Plus, 14.0, palette.accent),
                            text("Add").size(12).color(palette.accent)
                        ]
                        .spacing(7)
                        .align_y(iced::Alignment::Center)
                    )
                    .padding([12, 11])
                    .style(secondary_button_style(palette))
                    .on_press(Message::AddDiscoveryDirectory),
                ]
                .spacing(9)
                .align_y(iced::Alignment::End),
                directories,
                row![
                    iced::widget::Space::new().width(Length::Fill),
                    button(text("Save discovery settings").size(12).color(Color::WHITE))
                        .padding([12, 13])
                        .style(primary_button_style(palette))
                        .on_press(Message::SaveDiscoverySettings),
                ],
            ]
            .spacing(13),
            palette,
        ),
        card(
            column![
                text("Appearance").size(17).color(palette.text),
                row![
                    column![
                        text(format!(
                            "Current theme: {}",
                            if app.dark_mode { "Dark" } else { "Light" }
                        ))
                        .size(12)
                        .color(palette.secondary),
                        text("Choose a light or dark appearance for Devbox.")
                            .size(11)
                            .color(palette.secondary),
                    ]
                    .spacing(3),
                    iced::widget::Space::new().width(Length::Fill),
                    button(
                        row![
                            icon(
                                if app.dark_mode {
                                    AppIcon::Light
                                } else {
                                    AppIcon::Dark
                                },
                                15.0,
                                palette.secondary,
                            ),
                            text(if app.dark_mode {
                                "Switch to light"
                            } else {
                                "Switch to dark"
                            })
                            .size(12),
                        ]
                        .spacing(7)
                        .align_y(iced::Alignment::Center),
                    )
                    .padding([9, 12])
                    .style(secondary_button_style(palette))
                    .on_press(Message::ToggleTheme),
                ]
                .spacing(12)
                .align_y(iced::Alignment::Center),
            ]
            .spacing(12),
            palette,
        ),
        card(
            column![
                text("Ownership and network safety").size(17).color(palette.text),
                text("Host services found during scans are read-only. Devbox can stop only child process groups it started itself.")
                    .size(12).color(palette.secondary),
                text("Managed service data directories must already exist and belong to your user. Ports below 1024 are rejected.")
                    .size(12).color(palette.secondary),
                text("Local proxy routes are limited to .localhost domains and loopback upstreams. Config updates validate first and preserve a .devbox-backup file; Caddy reload remains under your service manager.")
                    .size(12).color(palette.secondary),
                text("Extension manifests are validated and capability approval is recorded. This Devbox version does not run extension code.")
                    .size(12).color(palette.secondary),
                text("Keyboard: Ctrl/⌘ + 1–9 switches pages; Tab moves between controls. Minimum window size is 860 × 600.")
                    .size(12).color(palette.secondary),
            ].spacing(9),
            palette,
        ),
    ]
    .spacing(14);
    scroll_area(content, Length::Fill)
}

fn services_page(app: &App, palette: Colors) -> Element<'_, Message> {
    let mut content = column![
        card(
            column![
                row![
                    icon(AppIcon::Services, 18.0, palette.accent),
                    text("Devbox-managed service profiles").size(16).color(palette.text)
                ]
                .spacing(9)
                .align_y(iced::Alignment::Center),
                text("A managed profile launches only the executable and process group that you explicitly configured here. Detected host services below remain read-only; Devbox never stops an external process.")
                    .size(12)
                    .color(palette.secondary),
            ]
            .spacing(9),
            palette,
        ),
    ]
    .spacing(13);
    content = content.push(card(
        column![
            text("Create a managed service profile")
                .size(17)
                .color(palette.text),
            form_group(
                column![
                    form_group_heading(
                        AppIcon::Services,
                        "Launch configuration",
                        "Choose the service type, executable, and arguments.",
                        palette,
                    ),
                    row![
                        form_field(
                            "Service name",
                            text_input("e.g. Local PostgreSQL", &app.service_form.name)
                                .on_input(Message::ServiceNameChanged)
                                .padding([9, 11])
                                .style(text_input_style(palette))
                                .width(Length::Fill),
                            Length::FillPortion(2),
                            palette,
                        ),
                        form_field(
                            "Service type",
                            pick_list(
                                vec![
                                    "Database".to_string(),
                                    "Cache".to_string(),
                                    "Email".to_string(),
                                    "Object storage".to_string(),
                                    "Web server".to_string(),
                                    "Other".to_string(),
                                ],
                                Some(app.service_form.kind.clone()),
                                Message::ServiceKindChanged,
                            )
                            .padding([9, 11])
                            .style(pick_list_style(palette))
                            .menu_style(pick_menu_style(palette))
                            .width(Length::Fill),
                            Length::FillPortion(1),
                            palette,
                        ),
                    ]
                    .spacing(9),
                    form_field(
                        "Server executable",
                        row![
                            text_input(
                                "Path to an existing executable",
                                &app.service_form.executable
                            )
                            .on_input(Message::ServiceExecutableChanged)
                            .padding([9, 11])
                            .style(text_input_style(palette))
                            .width(Length::Fill),
                            button(
                                row![
                                    icon(AppIcon::OpenFolder, 14.0, palette.secondary),
                                    text("Browse").size(12)
                                ]
                                .spacing(7)
                                .align_y(iced::Alignment::Center)
                            )
                            .padding([12, 11])
                            .style(secondary_button_style(palette))
                            .on_press(Message::PickServiceExecutable),
                        ]
                        .spacing(8)
                        .align_y(iced::Alignment::Center),
                        Length::Fill,
                        palette,
                    ),
                    form_field(
                        "Arguments",
                        text_input(
                            "Passed directly to the executable; no shell is used",
                            &app.service_form.arguments
                        )
                        .on_input(Message::ServiceArgumentsChanged)
                        .padding([9, 11])
                        .style(text_input_style(palette))
                        .width(Length::Fill),
                        Length::Fill,
                        palette,
                    ),
                ]
                .spacing(10),
                palette,
            ),
            form_group(
                column![
                    form_group_heading(
                        AppIcon::Database,
                        "Data and readiness",
                        "Use an existing directory you own. Ports below 1024 cannot be managed.",
                        palette,
                    ),
                    form_field(
                        "Data directory (optional)",
                        row![
                            text_input(
                                "Existing directory owned by your user",
                                &app.service_form.data_directory
                            )
                            .on_input(Message::ServiceDataDirectoryChanged)
                            .padding([9, 11])
                            .style(text_input_style(palette))
                            .width(Length::Fill),
                            button(
                                row![
                                    icon(AppIcon::OpenFolder, 14.0, palette.secondary),
                                    text("Browse").size(12)
                                ]
                                .spacing(7)
                                .align_y(iced::Alignment::Center)
                            )
                            .padding([12, 11])
                            .style(secondary_button_style(palette))
                            .on_press(Message::PickServiceDataDirectory),
                        ]
                        .spacing(8)
                        .align_y(iced::Alignment::Center),
                        Length::Fill,
                        palette,
                    ),
                    form_field(
                        "TCP ports (optional)",
                        text_input(
                            "Comma-separated unprivileged ports",
                            &app.service_form.ports
                        )
                        .on_input(Message::ServicePortsChanged)
                        .padding([9, 11])
                        .style(text_input_style(palette))
                        .width(Length::Fill),
                        Length::Fill,
                        palette,
                    ),
                ]
                .spacing(10),
                palette,
            ),
            row![
                text("Devbox does not initialize database directories.")
                    .size(11)
                    .color(palette.secondary),
                iced::widget::Space::new().width(Length::Fill),
                button(text("Save service profile").size(12).color(Color::WHITE))
                    .padding([12, 13])
                    .style(primary_button_style(palette))
                    .on_press(Message::SaveServiceProfile),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        ]
        .spacing(12),
        palette,
    ));
    for profile in &app.managed_service_profiles {
        let state = app
            .managed_service_states
            .iter()
            .find(|state| state.service_id == profile.id);
        let running =
            state.is_some_and(|state| state.status == "running" || state.status == "stopping");
        let status = state
            .map(|state| format!("{} · PID {} · {}", state.status, state.pid, state.health))
            .unwrap_or_else(|| "Stopped · profile saved".into());
        let action: Element<'_, Message> = if running {
            row![
                button(text("Restart").size(11))
                    .padding([7, 9])
                    .style(secondary_button_style(palette))
                    .on_press(Message::RestartManagedService(profile.id.clone())),
                button(text("Stop").size(11))
                    .padding([7, 9])
                    .style(secondary_button_style(palette))
                    .on_press(Message::StopManagedService(profile.id.clone())),
            ]
            .spacing(6)
            .into()
        } else {
            button(text("Start").size(11).color(Color::WHITE))
                .padding([7, 9])
                .style(primary_button_style(palette))
                .on_press(Message::StartManagedService(profile.id.clone()))
                .into()
        };
        let mut details = column![
            text(format!(
                "{} · {} · Devbox-owned",
                profile.name, profile.kind
            ))
            .size(14)
            .color(palette.text),
            text(status).size(11).color(if running {
                palette.success
            } else {
                palette.secondary
            }),
            text(format!(
                "{} {}",
                profile.executable_path,
                profile.arguments.join(" ")
            ))
            .size(10)
            .color(palette.secondary),
            text(if profile.ports.is_empty() {
                "No health ports configured".into()
            } else {
                format!(
                    "TCP {}",
                    profile
                        .ports
                        .iter()
                        .map(u16::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
            .size(10)
            .color(palette.secondary),
        ]
        .spacing(4);
        if let Some(state) = state {
            for log in state.logs.iter().rev().take(4) {
                details = details.push(text(log).size(10).color(palette.secondary));
            }
        }
        content = content.push(card(
            row![
                icon(AppIcon::Services, 21.0, palette.accent),
                details.width(Length::Fill),
                action,
                button(text("Remove").size(11).color(palette.error))
                    .padding([7, 9])
                    .style(secondary_button_style(palette))
                    .on_press(Message::RemoveServiceProfile(profile.id.clone())),
            ]
            .spacing(10)
            .align_y(iced::Alignment::Center),
            palette,
        ));
    }
    content = content.push(
        text("Detected host services · read only")
            .size(16)
            .color(palette.text),
    );
    if app.services.is_empty() {
        content = content.push(empty_state(
            AppIcon::Services,
            "No service scan data",
            "Refresh to inspect known local service binaries and endpoints.",
            palette,
        ));
    }
    for service in &app.services {
        let icon_name = match service.category.as_str() {
            "Database" => AppIcon::Database,
            "Cache" => AppIcon::Activity,
            "Web server" => AppIcon::Services,
            "Email" => AppIcon::Services,
            _ => AppIcon::Binary,
        };
        let state_detail = match &service.state {
            ServiceState::Running => format!(
                "Detected {} matching process{} · owner not verified",
                service.process_count,
                if service.process_count == 1 { "" } else { "es" }
            ),
            ServiceState::EndpointReachable => format!(
                "TCP {} responds · process owner not identified",
                service.reachable_port.unwrap_or_default()
            ),
            ServiceState::InstalledStopped => {
                "Server binary found · no matching process or endpoint".into()
            }
            ServiceState::Unhealthy => format!(
                "Matching process found · expected port{} {} not reachable",
                if service.endpoint_ports.len() == 1 {
                    ""
                } else {
                    "s"
                },
                service
                    .endpoint_ports
                    .iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            ServiceState::Missing => "No server binary or local endpoint detected".into(),
        };
        let mut facts = vec![format!(
            "{} · {} · external ownership unknown · read-only",
            service.category, service.discovery_source,
        )];
        if let Some(path) = &service.executable_path {
            facts.push(path.clone());
        }
        if !service.endpoint_ports.is_empty() {
            facts.push(format!(
                "Expected ports: {}",
                service
                    .endpoint_ports
                    .iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let card_content = row![
            icon(icon_name, 22.0, palette.accent),
            column![
                text(&service.name).size(14).color(palette.text),
                text(state_detail)
                    .size(12)
                    .color(service_state_color(&service.state, palette)),
                column(
                    facts
                        .into_iter()
                        .map(|fact| text(fact).size(10).color(palette.secondary).into())
                )
                .spacing(3),
            ]
            .spacing(5),
            iced::widget::Space::new().width(Length::Fill),
            service_state_dot(&service.state, palette),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center);
        content = content.push(card(card_content, palette));
    }
    scroll_area(content, Length::Fill)
}

fn workspaces_page(app: &App, palette: Colors) -> Element<'_, Message> {
    let project_choices = app
        .projects
        .iter()
        .map(|project| ProjectChoice {
            id: project.id.clone(),
            label: project.name.clone(),
        })
        .collect::<Vec<_>>();
    let workspace_choices = app
        .workspaces
        .iter()
        .map(|workspace| WorkspaceChoice {
            id: workspace.id.clone(),
            label: workspace.name.clone(),
        })
        .collect::<Vec<_>>();
    let member_ids = app
        .workspace_form
        .workspace
        .as_ref()
        .and_then(|selected| {
            app.workspaces
                .iter()
                .find(|workspace| workspace.id == selected.id)
        })
        .map(|workspace| workspace.project_ids.clone())
        .unwrap_or_default();
    let member_choices = app
        .projects
        .iter()
        .filter(|project| member_ids.contains(&project.id))
        .map(|project| ProjectChoice {
            id: project.id.clone(),
            label: project.name.clone(),
        })
        .collect::<Vec<_>>();
    let service_choices = app
        .managed_service_profiles
        .iter()
        .map(|profile| profile.id.clone())
        .collect::<Vec<_>>();
    let mut content = column![
        card(column![
            text("Create a workspace").size(17).color(palette.text),
            text("Workspaces group existing project directories. Startup follows prerequisites, and a partial failure stops projects started during that attempt.").size(12).color(palette.secondary),
            row![
                form_field(
                    "Workspace name",
                    text_input("e.g. Local storefront", &app.workspace_form.name)
                        .on_input(Message::WorkspaceNameChanged)
                        .padding([9, 11])
                        .style(text_input_style(palette))
                        .width(Length::Fill),
                    Length::Fill,
                    palette,
                ),
                button(text("Create workspace").size(12).color(Color::WHITE))
                    .padding([12, 13])
                    .style(primary_button_style(palette))
                    .on_press(Message::CreateWorkspace),
            ]
            .spacing(9)
            .align_y(iced::Alignment::End),
        ].spacing(11), palette),
        card(column![
            text("Configure workspace").size(16).color(palette.text),
            text("Add members, set startup order, and attach shared services.")
                .size(12)
                .color(palette.secondary),
            form_field(
                "Workspace",
                pick_list(workspace_choices.clone(), app.workspace_form.workspace.clone(), Message::WorkspaceSelected)
                    .placeholder("Choose a workspace to manage")
                    .padding([9, 11])
                    .style(pick_list_style(palette))
                    .menu_style(pick_menu_style(palette))
                    .width(Length::Fill),
                Length::Fill,
                palette,
            ),
            text(if app.workspace_form.workspace.is_some() {
                "Add projects, then set prerequisites and shared services for this workspace."
            } else if app.workspaces.is_empty() {
                "Create a workspace above before adding projects, dependencies, or services."
            } else {
                "Select a workspace before adding projects, dependencies, or shared services."
            })
            .size(11)
            .color(palette.secondary),
            form_group(
                column![
                    form_group_heading(
                        AppIcon::Projects,
                        "Projects",
                        "Add registered projects to this workspace.",
                        palette,
                    ),
                    row![
                        form_field(
                            "Project to add",
                            pick_list(project_choices, app.workspace_form.project.clone(), Message::WorkspaceProjectSelected)
                                .placeholder("Choose a registered project")
                                .padding([9, 11])
                                .style(pick_list_style(palette))
                                .menu_style(pick_menu_style(palette))
                                .width(Length::Fill),
                            Length::Fill,
                            palette,
                        ),
                        button(
                            row![
                                icon(AppIcon::Plus, 14.0, palette.accent),
                                text("Add project").size(12).color(palette.accent)
                            ]
                            .spacing(7)
                            .align_y(iced::Alignment::Center)
                        )
                        .padding([12, 13])
                        .style(secondary_button_style(palette))
                        .on_press(Message::AddWorkspaceProject),
                    ]
                    .spacing(9)
                    .align_y(iced::Alignment::End),
                ]
                .spacing(10),
                palette,
            ),
            form_group(
                column![
                    form_group_heading(
                        AppIcon::Activity,
                        "Startup dependencies",
                        "The dependent project waits for its prerequisite to be ready.",
                        palette,
                    ),
                    row![
                        form_field(
                            "Dependent project",
                            pick_list(member_choices.clone(), app.workspace_form.dependency_project.clone(), Message::DependencyProjectSelected)
                                .placeholder("Choose a member")
                                .padding([9, 11])
                                .style(pick_list_style(palette))
                                .menu_style(pick_menu_style(palette))
                                .width(Length::Fill),
                            Length::FillPortion(1),
                            palette,
                        ),
                        form_field(
                            "Required first",
                            pick_list(member_choices, app.workspace_form.prerequisite.clone(), Message::PrerequisiteSelected)
                                .placeholder("Choose its prerequisite")
                                .padding([9, 11])
                                .style(pick_list_style(palette))
                                .menu_style(pick_menu_style(palette))
                                .width(Length::Fill),
                            Length::FillPortion(1),
                            palette,
                        ),
                        button(
                            row![
                                icon(AppIcon::Plus, 14.0, palette.accent),
                                text("Add dependency").size(12).color(palette.accent)
                            ]
                            .spacing(7)
                            .align_y(iced::Alignment::Center)
                        )
                        .padding([12, 13])
                        .style(secondary_button_style(palette))
                        .on_press(Message::AddWorkspaceDependency),
                    ]
                    .spacing(9)
                    .align_y(iced::Alignment::End),
                ]
                .spacing(10),
                palette,
            ),
            form_group(
                column![
                    form_group_heading(
                        AppIcon::Services,
                        "Shared services",
                        "Attach a Devbox-managed service profile to this workspace.",
                        palette,
                    ),
                    row![
                        form_field(
                            "Managed service profile",
                            pick_list(service_choices, app.workspace_form.service.clone(), Message::WorkspaceServiceSelected)
                                .placeholder("Choose a service profile")
                                .padding([9, 11])
                                .style(pick_list_style(palette))
                                .menu_style(pick_menu_style(palette))
                                .width(Length::Fill),
                            Length::Fill,
                            palette,
                        ),
                        button(
                            row![
                                icon(AppIcon::Services, 14.0, palette.accent),
                                text("Share service").size(12).color(palette.accent)
                            ]
                            .spacing(7)
                            .align_y(iced::Alignment::Center)
                        )
                        .padding([12, 13])
                        .style(secondary_button_style(palette))
                        .on_press(Message::AttachWorkspaceService),
                    ]
                    .spacing(9)
                    .align_y(iced::Alignment::End),
                ]
                .spacing(10),
                palette,
            ),
        ].spacing(10), palette),
    ].spacing(13);

    if app.workspaces.is_empty() {
        content = content.push(empty_state(
            AppIcon::Workspaces,
            "No workspaces yet",
            "Create a workspace, add project memberships, and set dependencies before starting it.",
            palette,
        ));
    }
    for workspace in &app.workspaces {
        let names = workspace
            .project_ids
            .iter()
            .filter_map(|id| {
                app.projects
                    .iter()
                    .find(|project| &project.id == id)
                    .map(|project| project.name.clone())
            })
            .collect::<Vec<_>>();
        let active = app
            .processes
            .iter()
            .filter(|process| {
                workspace.project_ids.contains(&process.project_id)
                    && process.status == ProcessStatus::Running
            })
            .count();
        let linked_services = workspace
            .service_ids
            .iter()
            .filter_map(|id| {
                app.managed_service_profiles
                    .iter()
                    .find(|profile| &profile.id == id)
                    .map(|profile| profile.name.clone())
            })
            .collect::<Vec<_>>();
        let dependency_lines = workspace
            .dependencies
            .iter()
            .flat_map(|(project_id, requires)| {
                requires.iter().map(move |required| {
                    let name = |id: &str| {
                        app.projects
                            .iter()
                            .find(|project| project.id == id)
                            .map(|project| project.name.as_str())
                            .unwrap_or("Missing project")
                    };
                    format!("{} waits for {}", name(project_id), name(required))
                })
            })
            .collect::<Vec<_>>();
        let mut facts = column![
            text(format!(
                "{} of {} projects running",
                active,
                workspace.project_ids.len()
            ))
            .size(12)
            .color(palette.secondary),
            text(if names.is_empty() {
                "No project members".to_string()
            } else {
                names.join(" · ")
            })
            .size(12)
            .color(palette.text),
            text(if linked_services.is_empty() {
                "No shared managed services".to_string()
            } else {
                format!("Shared services: {}", linked_services.join(" · "))
            })
            .size(11)
            .color(palette.secondary),
        ]
        .spacing(5);
        for dependency in dependency_lines {
            facts = facts.push(text(dependency).size(11).color(palette.secondary));
        }
        let recent_logs = app
            .processes
            .iter()
            .filter(|process| workspace.project_ids.contains(&process.project_id))
            .flat_map(|process| {
                process
                    .logs
                    .iter()
                    .rev()
                    .take(2)
                    .map(move |line| format!("{} · {}", process.project_name, line))
            })
            .take(4)
            .collect::<Vec<_>>();
        if !recent_logs.is_empty() {
            facts = facts.push(
                text(recent_logs.join("\n"))
                    .size(10)
                    .color(palette.secondary),
            );
        }
        content = content.push(card(
            row![
                icon(AppIcon::Workspaces, 22.0, palette.accent),
                column![text(&workspace.name).size(15).color(palette.text), facts]
                    .spacing(7)
                    .width(Length::Fill),
                button(text("Start all").size(11).color(Color::WHITE))
                    .padding([8, 10])
                    .style(primary_button_style(palette))
                    .on_press(Message::StartWorkspace(workspace.id.clone())),
                button(text("Stop all").size(11))
                    .padding([8, 10])
                    .style(secondary_button_style(palette))
                    .on_press(Message::StopWorkspace(workspace.id.clone())),
            ]
            .spacing(10)
            .align_y(iced::Alignment::Center),
            palette,
        ));
    }
    scroll_area(content, Length::Fill)
}

fn extensions_page(app: &App, palette: Colors) -> Element<'_, Message> {
    let mut content = column![
        card(column![
            row![icon(AppIcon::Extensions, 19.0, palette.accent), text("Register an extension manifest").size(17).color(palette.text)].spacing(9).align_y(iced::Alignment::Center),
            text("Devbox validates devbox-extension.json files and records declared capabilities. The current API does not execute extension code.").size(12).color(palette.secondary),
            row![
                form_field(
                    "Manifest file",
                    row![
                        text_input("Path to devbox-extension.json", &app.extension_manifest_path)
                            .on_input(Message::ExtensionManifestChanged)
                            .padding([9, 11])
                            .style(text_input_style(palette))
                            .width(Length::Fill),
                        button(text("Browse").size(12))
                            .padding([12, 13])
                            .style(secondary_button_style(palette))
                            .on_press(Message::PickExtensionManifest),
                    ]
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
                    Length::Fill,
                    palette,
                ),
                button(text("Register").size(12).color(Color::WHITE))
                    .padding([12, 13])
                    .style(primary_button_style(palette))
                    .on_press(Message::RegisterExtension),
            ]
            .spacing(8)
            .align_y(iced::Alignment::End),
        ].spacing(10), palette),
    ].spacing(13);
    if app.extensions.is_empty() {
        content = content.push(empty_state(
            AppIcon::Extensions,
            "No registered extensions",
            "Only validated manifests are listed; registering one never runs its code.",
            palette,
        ));
    }
    for extension in &app.extensions {
        let permissions = if extension.capabilities.is_empty() {
            "No capabilities requested".to_string()
        } else {
            format!("Requests: {}", extension.capabilities.join(", "))
        };
        let approved = if extension.approved_capabilities.is_empty() {
            "No capabilities approved".to_string()
        } else {
            format!("Approved: {}", extension.approved_capabilities.join(", "))
        };
        let action = if extension.enabled {
            button(text("Disable").size(11))
                .padding([7, 9])
                .style(secondary_button_style(palette))
                .on_press(Message::DisableExtension(extension.id.clone()))
        } else {
            button(text("Approve & enable").size(11).color(Color::WHITE))
                .padding([7, 9])
                .style(primary_button_style(palette))
                .on_press(Message::ApproveExtension(extension.id.clone()))
        };
        content = content.push(card(
            row![
                icon(AppIcon::Extensions, 21.0, palette.accent),
                column![
                    text(format!("{} · {}", extension.name, extension.version))
                        .size(14)
                        .color(palette.text),
                    text(format!(
                        "{} · API {} · {}",
                        if extension.enabled {
                            "Enabled in registry"
                        } else {
                            "Disabled"
                        },
                        extension.api_version,
                        permissions
                    ))
                    .size(11)
                    .color(palette.secondary),
                    text(approved).size(11).color(palette.secondary),
                    text(&extension.manifest_path)
                        .size(10)
                        .color(palette.secondary),
                ]
                .spacing(4)
                .width(Length::Fill),
                action,
                button(text("Remove").size(11).color(palette.error))
                    .padding([7, 9])
                    .style(secondary_button_style(palette))
                    .on_press(Message::RemoveExtension(extension.id.clone())),
            ]
            .spacing(10)
            .align_y(iced::Alignment::Center),
            palette,
        ));
    }
    scroll_area(content, Length::Fill)
}

fn networking_page(app: &App, palette: Colors) -> Element<'_, Message> {
    let project_choices = app
        .projects
        .iter()
        .map(|project| ProjectChoice {
            id: project.id.clone(),
            label: project.name.clone(),
        })
        .collect::<Vec<_>>();
    let mut routes = column![].spacing(8);
    if app.local_domains.is_empty() {
        routes = routes.push(
            text("No local domains configured")
                .size(12)
                .color(palette.secondary),
        );
    }
    for route in &app.local_domains {
        let project_name = app
            .projects
            .iter()
            .find(|project| project.id == route.project_id)
            .map(|project| project.name.as_str())
            .unwrap_or("Missing project");
        routes = routes.push(
            row![
                icon(AppIcon::Services, 16.0, palette.accent),
                text(format!(
                    "{} → {} ({})",
                    route.domain, route.upstream, project_name
                ))
                .size(12)
                .color(palette.text),
                iced::widget::Space::new().width(Length::Fill),
                button(text("Remove").size(10).color(palette.error))
                    .padding([5, 8])
                    .style(secondary_button_style(palette))
                    .on_press(Message::RemoveDomain(route.id.clone())),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        );
    }
    let content = column![
        card(column![
            row![icon(AppIcon::Services, 19.0, palette.accent), text("Project domains").size(17).color(palette.text)].spacing(9).align_y(iced::Alignment::Center),
            text("Routes are restricted to *.localhost domains and loopback upstreams. Saving a route only changes Devbox data; applying writes a Caddyfile to a path you choose and keeps a backup. It never edits hosts or certificate stores.").size(12).color(palette.secondary),
            form_field(
                "Project",
                pick_list(project_choices, app.domain_form.project.clone(), Message::DomainProjectSelected)
                    .placeholder("Choose a registered project")
                    .padding([9, 11])
                    .style(pick_list_style(palette))
                    .menu_style(pick_menu_style(palette))
                    .width(Length::Fill),
                Length::Fill,
                palette,
            ),
            row![
                form_field(
                    "Local domain",
                    text_input("e.g. api.localhost", &app.domain_form.domain)
                        .on_input(Message::DomainNameChanged)
                        .padding([9, 11])
                        .style(text_input_style(palette))
                        .width(Length::Fill),
                    Length::FillPortion(1),
                    palette,
                ),
                form_field(
                    "Loopback upstream",
                    text_input("e.g. http://127.0.0.1:8080", &app.domain_form.upstream)
                        .on_input(Message::DomainUpstreamChanged)
                        .padding([9, 11])
                        .style(text_input_style(palette))
                        .width(Length::Fill),
                    Length::FillPortion(2),
                    palette,
                ),
            ]
            .spacing(9),
            checkbox(app.domain_form.websocket)
                .label("This upstream uses WebSockets")
                .style(checkbox_style(palette))
                .on_toggle(Message::DomainWebsocketChanged),
            row![
                button(text("Save domain").size(12).color(Color::WHITE))
                    .padding([12, 13])
                    .style(primary_button_style(palette))
                    .on_press(Message::SaveDomain),
                button(text("Preview Caddyfile").size(12))
                    .padding([12, 13])
                    .style(secondary_button_style(palette))
                    .on_press(Message::RenderProxyConfig),
            ].spacing(8),
            routes,
        ].spacing(11), palette),
        card(column![
            text("Validate and apply a user-owned Caddyfile").size(16).color(palette.text),
            text("Caddy validation runs before the selected file is replaced. Existing contents are preserved as <filename>.devbox-backup. Devbox does not reload Caddy; review and reload it through the service manager you use.").size(12).color(palette.secondary),
            form_field(
                "Caddy executable",
                text_input("Path or command name", &app.domain_form.caddy_executable)
                    .on_input(Message::CaddyExecutableChanged)
                    .padding([9, 11])
                    .style(text_input_style(palette))
                    .width(Length::Fill),
                Length::Fill,
                palette,
            ),
            form_field(
                "Caddyfile destination",
                row![
                    text_input("Choose where to write the validated file", &app.domain_form.config_target)
                        .on_input(Message::ProxyTargetChanged)
                        .padding([9, 11])
                        .style(text_input_style(palette))
                        .width(Length::Fill),
                    button(text("Choose file").size(12))
                        .padding([12, 13])
                        .style(secondary_button_style(palette))
                        .on_press(Message::PickProxyTarget),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
                Length::Fill,
                palette,
            ),
            row![
                iced::widget::Space::new().width(Length::Fill),
                button(text("Restore backup").size(12))
                    .padding([12, 13])
                    .style(secondary_button_style(palette))
                    .on_press(Message::RestoreProxyConfig),
                button(text("Validate & write").size(12).color(Color::WHITE))
                    .padding([12, 13])
                    .style(primary_button_style(palette))
                    .on_press(Message::ApplyProxyConfig),
            ]
            .spacing(8),
            text(if app.proxy_preview.is_empty() { "Preview the generated HTTP routes before applying." } else { &app.proxy_preview }).size(11).color(palette.secondary),
        ].spacing(10), palette),
    ].spacing(13);
    scroll_area(content, Length::Fill)
}

fn empty_state(
    icon_name: AppIcon,
    title: &str,
    message: &str,
    palette: Colors,
) -> Element<'static, Message> {
    card(
        column![
            icon(icon_name, 36.0, palette.accent),
            text(title.to_string())
                .size(16)
                .color(palette.text)
                .width(Length::Fill)
                .align_x(iced::alignment::Horizontal::Center),
            text(message.to_string())
                .size(12)
                .color(palette.secondary)
                .width(Length::Fill)
                .align_x(iced::alignment::Horizontal::Center),
        ]
        .spacing(11)
        .align_x(iced::Alignment::Center),
        palette,
    )
}

fn action_button(
    icon_name: AppIcon,
    label: &str,
    message: Message,
    palette: Colors,
) -> Element<'static, Message> {
    button(
        row![
            icon(icon_name, 15.0, palette.accent),
            text(label.to_string()).size(12).color(palette.accent)
        ]
        .spacing(7)
        .align_y(iced::Alignment::Center),
    )
    .padding([9, 11])
    .style(secondary_button_style(palette))
    .on_press(message)
    .into()
}

fn card<'a>(content: impl Into<Element<'a, Message>>, palette: Colors) -> Element<'a, Message> {
    container(content)
        .padding(16)
        .width(Length::Fill)
        .style(move |_| container::Style {
            background: Some(Background::Color(palette.surface)),
            border: Border {
                color: palette.surface_border,
                width: 1.0,
                radius: 11.0.into(),
            },
            ..Default::default()
        })
        .into()
}

fn form_field<'a>(
    label: &'static str,
    control: impl Into<Element<'a, Message>>,
    width: Length,
    palette: Colors,
) -> Element<'a, Message> {
    let control: Element<'a, Message> = control.into();
    column![text(label).size(11).color(palette.secondary), control,]
        .spacing(5)
        .width(width)
        .into()
}

fn form_group<'a>(
    content: impl Into<Element<'a, Message>>,
    palette: Colors,
) -> Element<'a, Message> {
    container(content)
        .padding(12)
        .width(Length::Fill)
        .style(move |_| container::Style {
            background: Some(Background::Color(palette.background)),
            border: Border {
                color: palette.surface_border,
                width: 1.0,
                radius: 10.0.into(),
            },
            ..Default::default()
        })
        .into()
}

fn form_group_heading(
    icon_name: AppIcon,
    title: &'static str,
    description: &'static str,
    palette: Colors,
) -> Element<'static, Message> {
    row![
        icon(icon_name, 17.0, palette.accent),
        column![
            text(title).size(13).color(palette.text),
            text(description).size(11).color(palette.secondary),
        ]
        .spacing(2),
    ]
    .spacing(9)
    .align_y(iced::Alignment::Center)
    .into()
}

fn text_input_style(
    palette: Colors,
) -> impl Fn(&Theme, iced::widget::text_input::Status) -> iced::widget::text_input::Style {
    move |_, status| {
        let focused = matches!(status, iced::widget::text_input::Status::Focused { .. });
        let hovered = matches!(status, iced::widget::text_input::Status::Hovered);
        let disabled = matches!(status, iced::widget::text_input::Status::Disabled);
        iced::widget::text_input::Style {
            background: Background::Color(if disabled {
                palette.background
            } else {
                palette.surface
            }),
            border: Border {
                color: if focused {
                    palette.accent
                } else if hovered {
                    palette.separator
                } else {
                    palette.surface_border
                },
                width: if focused { 1.5 } else { 1.0 },
                radius: 8.0.into(),
            },
            icon: palette.secondary,
            placeholder: palette.secondary,
            value: palette.text,
            selection: palette.nav_selected,
        }
    }
}

fn pick_list_style(
    palette: Colors,
) -> impl Fn(&Theme, iced::widget::pick_list::Status) -> iced::widget::pick_list::Style {
    move |_, status| {
        let open = matches!(status, iced::widget::pick_list::Status::Opened { .. });
        let hovered = matches!(status, iced::widget::pick_list::Status::Hovered);
        iced::widget::pick_list::Style {
            text_color: palette.text,
            placeholder_color: palette.secondary,
            handle_color: palette.secondary,
            background: Background::Color(palette.surface),
            border: Border {
                color: if open {
                    palette.accent
                } else if hovered {
                    palette.separator
                } else {
                    palette.surface_border
                },
                width: if open { 1.5 } else { 1.0 },
                radius: 8.0.into(),
            },
        }
    }
}

fn pick_menu_style(palette: Colors) -> impl Fn(&Theme) -> iced::widget::overlay::menu::Style {
    move |_| iced::widget::overlay::menu::Style {
        background: Background::Color(palette.surface),
        border: Border {
            color: palette.surface_border,
            width: 1.0,
            radius: 8.0.into(),
        },
        text_color: palette.text,
        selected_text_color: palette.text,
        selected_background: Background::Color(palette.nav_selected),
        shadow: iced::Shadow::default(),
    }
}

fn checkbox_style(
    palette: Colors,
) -> impl Fn(&Theme, iced::widget::checkbox::Status) -> iced::widget::checkbox::Style {
    move |_, status| {
        let checked = match status {
            iced::widget::checkbox::Status::Active { is_checked }
            | iced::widget::checkbox::Status::Hovered { is_checked }
            | iced::widget::checkbox::Status::Disabled { is_checked } => is_checked,
        };
        let disabled = matches!(status, iced::widget::checkbox::Status::Disabled { .. });
        iced::widget::checkbox::Style {
            background: Background::Color(if checked {
                palette.accent
            } else {
                palette.surface
            }),
            icon_color: Color::WHITE,
            border: Border {
                color: if checked {
                    palette.accent
                } else {
                    palette.surface_border
                },
                width: 1.0,
                radius: 5.0.into(),
            },
            text_color: Some(if disabled {
                palette.secondary
            } else {
                palette.text
            }),
        }
    }
}

fn nav_button_style(palette: Colors, selected: bool, status: button::Status) -> button::Style {
    let background = if selected {
        Some(Background::Color(palette.nav_selected))
    } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
        Some(Background::Color(palette.nav_hover))
    } else {
        None
    };
    button::Style {
        text_color: palette.text,
        background,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 8.0.into(),
        },
        ..Default::default()
    }
}

fn primary_button_style(palette: Colors) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        text_color: Color::WHITE,
        background: Some(Background::Color(
            if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                palette.accent_hover
            } else {
                palette.accent
            },
        )),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 8.0.into(),
        },
        ..Default::default()
    }
}

fn secondary_button_style(palette: Colors) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        text_color: palette.text,
        background: Some(Background::Color(
            if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                palette.nav_hover
            } else {
                palette.surface
            },
        )),
        border: Border {
            color: palette.separator,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..Default::default()
    }
}

fn quiet_button_style(palette: Colors) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        text_color: palette.secondary,
        background: if matches!(status, button::Status::Hovered | button::Status::Pressed) {
            Some(Background::Color(palette.nav_hover))
        } else {
            None
        },
        border: Border::default(),
        ..Default::default()
    }
}

fn status_dot(status: &ProcessStatus, palette: Colors) -> Element<'static, Message> {
    let color = process_color(status, palette);
    container(
        iced::widget::Space::new()
            .width(Length::Fixed(8.0))
            .height(Length::Fixed(8.0)),
    )
    .style(move |_| container::Style {
        background: Some(Background::Color(color)),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 10.0.into(),
        },
        ..Default::default()
    })
    .into()
}

fn service_state_dot(state: &ServiceState, palette: Colors) -> Element<'static, Message> {
    let color = service_state_color(state, palette);
    container(
        iced::widget::Space::new()
            .width(Length::Fixed(8.0))
            .height(Length::Fixed(8.0)),
    )
    .style(move |_| container::Style {
        background: Some(Background::Color(color)),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 10.0.into(),
        },
        ..Default::default()
    })
    .into()
}

fn service_state_color(state: &ServiceState, palette: Colors) -> Color {
    match state {
        ServiceState::Running => palette.success,
        ServiceState::EndpointReachable => palette.accent,
        ServiceState::InstalledStopped => palette.secondary,
        ServiceState::Unhealthy => palette.warning,
        ServiceState::Missing => palette.secondary,
    }
}

fn process_color(status: &ProcessStatus, palette: Colors) -> Color {
    match status {
        ProcessStatus::Running => palette.success,
        ProcessStatus::Stopping => palette.warning,
        ProcessStatus::Exited => palette.secondary,
        ProcessStatus::Failed => palette.error,
    }
}

fn status_label(status: &ProcessStatus) -> &'static str {
    match status {
        ProcessStatus::Running => "Running",
        ProcessStatus::Stopping => "Stopping",
        ProcessStatus::Exited => "Stopped",
        ProcessStatus::Failed => "Failed",
    }
}

fn runtime_choices(binaries: &[BinaryRecord]) -> Vec<RuntimeChoice> {
    binaries
        .iter()
        .filter(|binary| {
            binary.enabled
                && matches!(
                    &binary.category,
                    BinaryCategory::Runtime | BinaryCategory::Tool
                )
        })
        .map(|binary| RuntimeChoice {
            id: binary.id.clone(),
            label: format!(
                "{} · {}",
                binary.display_name,
                binary.version().unwrap_or("unverified")
            ),
        })
        .collect()
}

fn selected_runtime_label(project: &Project, binaries: &[BinaryRecord]) -> String {
    project
        .runtime_binary_id
        .as_ref()
        .and_then(|id| binaries.iter().find(|binary| &binary.id == id))
        .map(|binary| format!("{} {}", binary.display_name, binary.version().unwrap_or("")))
        .unwrap_or_else(|| "Not selected".into())
}

fn set_status(app: &mut App, success: bool, message: String) {
    app.status = Some((success, message));
}

fn nonempty(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.trim().to_string())
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn format_time(seconds: i64) -> String {
    let now = unix_now();
    let age = now.saturating_sub(seconds);
    if age < 60 {
        format!("{age}s ago")
    } else if age < 3600 {
        format!("{}m ago", age / 60)
    } else {
        format!("{}h ago", age / 3600)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn app_with_core(temp: &TempDir) -> App {
        App {
            core: Some(Core::open(temp.path().join("devbox.sqlite3")).unwrap()),
            startup_error: None,
            binaries: Vec::new(),
            projects: Vec::new(),
            processes: Vec::new(),
            services: Vec::new(),
            managed_service_profiles: Vec::new(),
            managed_service_states: Vec::new(),
            workspaces: Vec::new(),
            extensions: Vec::new(),
            local_domains: Vec::new(),
            page: Page::Dashboard,
            dark_mode: false,
            busy: false,
            status: None,
            show_binary_form: false,
            show_project_form: false,
            binary_form: BinaryForm {
                category: "Runtime".into(),
                ..Default::default()
            },
            project_form: ProjectForm::default(),
            project_config_form: None,
            log_query: String::new(),
            log_stream: LogStreamChoice::All,
            binary_search_directories: Vec::new(),
            directory_entry: String::new(),
            automatic_rescan: true,
            php_reports: HashMap::new(),
            php_inspections: HashSet::new(),
            service_form: ServiceForm::default(),
            workspace_form: WorkspaceForm::default(),
            extension_manifest_path: String::new(),
            domain_form: DomainForm::default(),
            proxy_preview: String::new(),
        }
    }

    #[test]
    fn navigation_forms_busy_state_and_probe_errors_are_visible() {
        let temp = TempDir::new().unwrap();
        let mut app = app_with_core(&temp);

        let _ = update(&mut app, Message::Navigate(Page::Projects));
        assert_eq!(app.page, Page::Projects);
        let _ = update(&mut app, Message::ToggleProjectForm);
        assert!(app.show_project_form);

        let _ = update(&mut app, Message::AddProject);
        assert!(!app.busy);
        assert!(app.status.as_ref().is_some_and(|(success, message)| {
            !success && message.contains("Project name and directory are required")
        }));

        app.binary_form.path = "/path/that/does/not/exist".into();
        let _ = update(&mut app, Message::TestBinary);
        assert!(app.busy);
        let tested_path = app.binary_form.path.clone();
        let _ = update(
            &mut app,
            Message::BinaryTested(
                tested_path,
                ProbeResult {
                    version: None,
                    error: Some("version command could not be read".into()),
                },
            ),
        );
        assert!(!app.busy);
        assert!(app.status.as_ref().is_some_and(|(success, message)| {
            !success && message.contains("could not be read")
        }));

        let _view = view(&app);
    }

    #[test]
    fn every_management_page_builds_with_empty_and_populated_models() {
        let temp = TempDir::new().unwrap();
        let mut app = app_with_core(&temp);
        for page in [
            Page::Dashboard,
            Page::Projects,
            Page::Workspaces,
            Page::Binaries,
            Page::Processes,
            Page::Services,
            Page::Networking,
            Page::Extensions,
            Page::Settings,
        ] {
            app.page = page;
            let _view = view(&app);
        }
        app.workspaces.push(devbox_core::Workspace {
            id: "workspace".into(),
            name: "Example".into(),
            created_at: 0,
            project_ids: Vec::new(),
            dependencies: HashMap::new(),
            service_ids: Vec::new(),
        });
        app.extensions.push(devbox_core::ExtensionRecord {
            id: "example.extension".into(),
            name: "Example extension".into(),
            version: "1".into(),
            api_version: 1,
            manifest_path: "/tmp/devbox-extension.json".into(),
            capabilities: vec!["projects.read".into()],
            approved_capabilities: Vec::new(),
            enabled: false,
        });
        app.page = Page::Workspaces;
        let _view = view(&app);
        drop(_view);
        app.page = Page::Extensions;
        let _view = view(&app);
    }
}

fn format_elapsed(seconds: i64) -> String {
    let seconds = seconds.max(0);
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    if days > 0 {
        format!("{days}d {:02}h", hours % 24)
    } else if hours > 0 {
        format!("{hours}h {:02}m", minutes % 60)
    } else if minutes > 0 {
        format!("{minutes}m {:02}s", seconds % 60)
    } else {
        format!("{seconds}s")
    }
}

fn process_runtime_summary(
    process: &ProcessSnapshot,
    is_running: bool,
    visible_log_count: usize,
) -> String {
    if !is_running {
        return format!("{visible_log_count} log lines shown");
    }
    let cpu = process
        .cpu_percent
        .map(|percent| format!("{percent:.1}% CPU"))
        .unwrap_or_else(|| "CPU pending".into());
    let memory = process
        .memory_bytes
        .map(format_bytes)
        .map(|memory| format!("{memory} RAM"))
        .unwrap_or_else(|| "RAM pending".into());
    format!(
        "Running for {} · {cpu} · {memory}",
        format_elapsed(unix_now() - process.started_at)
    )
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[derive(Clone, Copy)]
struct Colors {
    background: Color,
    sidebar: Color,
    surface: Color,
    text: Color,
    secondary: Color,
    accent: Color,
    accent_hover: Color,
    success: Color,
    warning: Color,
    error: Color,
    separator: Color,
    surface_border: Color,
    nav_selected: Color,
    nav_border: Color,
    nav_hover: Color,
    log_background: Color,
    log_text: Color,
}

fn colors(dark: bool) -> Colors {
    if dark {
        Colors {
            background: Color::from_rgb8(28, 28, 30),
            sidebar: Color::from_rgb8(37, 37, 40),
            surface: Color::from_rgb8(44, 44, 46),
            text: Color::from_rgb8(245, 245, 247),
            secondary: Color::from_rgb8(152, 152, 157),
            accent: Color::from_rgb8(10, 132, 255),
            accent_hover: Color::from_rgb8(35, 148, 255),
            success: Color::from_rgb8(48, 209, 88),
            warning: Color::from_rgb8(255, 159, 10),
            error: Color::from_rgb8(255, 69, 58),
            separator: Color::from_rgb8(56, 56, 58),
            surface_border: Color::from_rgb8(64, 64, 68),
            nav_selected: Color::from_rgb8(42, 53, 68),
            nav_border: Color::from_rgb8(55, 71, 90),
            nav_hover: Color::from_rgb8(51, 51, 54),
            log_background: Color::from_rgb8(24, 24, 26),
            log_text: Color::from_rgb8(210, 210, 215),
        }
    } else {
        Colors {
            background: Color::from_rgb8(245, 245, 247),
            sidebar: Color::from_rgb8(233, 233, 239),
            surface: Color::WHITE,
            text: Color::from_rgb8(29, 29, 31),
            secondary: Color::from_rgb8(110, 110, 115),
            accent: Color::from_rgb8(0, 122, 255),
            accent_hover: Color::from_rgb8(0, 103, 220),
            success: Color::from_rgb8(52, 199, 89),
            warning: Color::from_rgb8(255, 149, 0),
            error: Color::from_rgb8(255, 59, 48),
            separator: Color::from_rgb8(210, 210, 215),
            surface_border: Color::from_rgb8(226, 226, 232),
            nav_selected: Color::from_rgb8(224, 233, 248),
            nav_border: Color::from_rgb8(204, 222, 248),
            nav_hover: Color::from_rgb8(224, 224, 230),
            log_background: Color::from_rgb8(248, 248, 250),
            log_text: Color::from_rgb8(50, 50, 55),
        }
    }
}
