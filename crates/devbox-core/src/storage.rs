use crate::model::{
    BinaryCategory, BinaryRecord, EnvironmentVariable, ExtensionRecord, LocalDomain,
    ManagedServiceProfile, ProcessSnapshot, ProcessStatus, Project, Workspace,
};
use anyhow::{Context, Result, anyhow};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Storage {
    path: PathBuf,
}

impl Storage {
    pub fn open_default() -> Result<Self> {
        let root = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
            .ok_or_else(|| anyhow!("neither XDG_DATA_HOME nor HOME is set"))?;
        Self::open(root.join("devbox/devbox.sqlite3"))
    }

    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let storage = Self { path: path.into() };
        if let Some(parent) = storage.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating database directory {}", parent.display()))?;
        }
        storage.migrate()?;
        Ok(storage)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn connection(&self) -> Result<Connection> {
        let connection = Connection::open(&self.path)
            .with_context(|| format!("opening database {}", self.path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&self.path)
                .with_context(|| format!("checking database permissions {}", self.path.display()))?
                .permissions();
            permissions.set_mode(0o600);
            std::fs::set_permissions(&self.path, permissions).with_context(|| {
                format!("protecting database permissions {}", self.path.display())
            })?;
        }
        connection.pragma_update(None, "foreign_keys", "ON")?;
        Ok(connection)
    }

    fn migrate(&self) -> Result<()> {
        let connection = self.connection()?;
        let current_version: i64 =
            connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if current_version > 5 {
            return Err(anyhow!(
                "database schema version {current_version} is newer than Devbox supports"
            ));
        }
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS detected_binaries (
                id TEXT PRIMARY KEY,
                display_name TEXT NOT NULL,
                category TEXT NOT NULL,
                executable_path TEXT NOT NULL UNIQUE,
                resolved_path TEXT,
                version_arguments TEXT NOT NULL,
                version_parser TEXT NOT NULL,
                version_regex TEXT,
                detected_version TEXT,
                probe_error TEXT,
                manual_version TEXT,
                discovery_source TEXT NOT NULL,
                trust_status TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                last_verified_at INTEGER
             );
             CREATE TABLE IF NOT EXISTS projects (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                path TEXT NOT NULL UNIQUE,
                runtime_binary_id TEXT REFERENCES detected_binaries(id) ON DELETE SET NULL,
                startup_arguments TEXT NOT NULL DEFAULT '[]',
                detected_frameworks TEXT NOT NULL DEFAULT '[]',
                environment_profile TEXT NOT NULL DEFAULT '[]',
                created_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS process_sessions (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                pid INTEGER NOT NULL,
                started_at INTEGER NOT NULL,
                status TEXT NOT NULL,
                exit_code INTEGER,
                error_detail TEXT
             );
             CREATE TABLE IF NOT EXISTS application_settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS managed_services (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                executable_path TEXT NOT NULL,
                arguments TEXT NOT NULL,
                data_directory TEXT,
                ports TEXT NOT NULL,
                created_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS workspaces (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                created_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS workspace_projects (
                workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
                project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                position INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(workspace_id, project_id)
             );
             CREATE TABLE IF NOT EXISTS project_dependencies (
                workspace_id TEXT NOT NULL,
                project_id TEXT NOT NULL,
                depends_on_project_id TEXT NOT NULL,
                CHECK(project_id <> depends_on_project_id),
                PRIMARY KEY(workspace_id, project_id, depends_on_project_id),
                FOREIGN KEY(workspace_id, project_id) REFERENCES workspace_projects(workspace_id, project_id) ON DELETE CASCADE,
                FOREIGN KEY(workspace_id, depends_on_project_id) REFERENCES workspace_projects(workspace_id, project_id) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS workspace_service_consumers (
                workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
                service_id TEXT NOT NULL REFERENCES managed_services(id) ON DELETE CASCADE,
                PRIMARY KEY(workspace_id, service_id)
             );
             CREATE TABLE IF NOT EXISTS local_domains (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                domain TEXT NOT NULL UNIQUE,
                upstream TEXT NOT NULL,
                websocket INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS extensions (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                version TEXT NOT NULL,
                api_version INTEGER NOT NULL,
                manifest_path TEXT NOT NULL UNIQUE,
                capabilities TEXT NOT NULL,
                approved_capabilities TEXT NOT NULL DEFAULT '[]',
                enabled INTEGER NOT NULL DEFAULT 0
             );
             ",
        )?;
        if !has_column(&connection, "projects", "detected_frameworks")? {
            connection.execute(
                "ALTER TABLE projects ADD COLUMN detected_frameworks TEXT NOT NULL DEFAULT '[]'",
                [],
            )?;
        }
        if !has_column(&connection, "projects", "environment_profile")? {
            connection.execute(
                "ALTER TABLE projects ADD COLUMN environment_profile TEXT NOT NULL DEFAULT '[]'",
                [],
            )?;
        }
        if !has_column(&connection, "detected_binaries", "probe_error")? {
            connection.execute(
                "ALTER TABLE detected_binaries ADD COLUMN probe_error TEXT",
                [],
            )?;
        }
        if !has_column(&connection, "process_sessions", "error_detail")? {
            connection.execute(
                "ALTER TABLE process_sessions ADD COLUMN error_detail TEXT",
                [],
            )?;
        }
        connection.pragma_update(None, "user_version", 5)?;
        Ok(())
    }

    pub fn list_binaries(&self) -> Result<Vec<BinaryRecord>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, display_name, category, executable_path, resolved_path,
                    version_arguments, version_parser, version_regex, detected_version,
                    manual_version, discovery_source, trust_status, enabled, last_verified_at,
                    probe_error
             FROM detected_binaries ORDER BY display_name COLLATE NOCASE, executable_path",
        )?;
        let rows = statement.query_map([], binary_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn save_binary(&self, binary: &BinaryRecord) -> Result<()> {
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO detected_binaries (
                id, display_name, category, executable_path, resolved_path,
                version_arguments, version_parser, version_regex, detected_version,
                manual_version, discovery_source, trust_status, enabled, last_verified_at,
                probe_error
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT(executable_path) DO UPDATE SET
                display_name = excluded.display_name,
                category = excluded.category,
                resolved_path = excluded.resolved_path,
                version_arguments = excluded.version_arguments,
                version_parser = excluded.version_parser,
                version_regex = excluded.version_regex,
                detected_version = excluded.detected_version,
                manual_version = excluded.manual_version,
                discovery_source = excluded.discovery_source,
                trust_status = excluded.trust_status,
                enabled = excluded.enabled,
                last_verified_at = excluded.last_verified_at,
                probe_error = excluded.probe_error",
            params![
                binary.id,
                binary.display_name,
                binary.category.as_str(),
                binary.executable_path,
                binary.resolved_path,
                serde_json::to_string(&binary.version_arguments)?,
                binary.version_parser,
                binary.version_regex,
                binary.detected_version,
                binary.manual_version,
                binary.discovery_source,
                binary.trust_status,
                binary.enabled,
                binary.last_verified_at,
                binary.probe_error,
            ],
        )?;
        Ok(())
    }

    pub fn save_discovered_binary(&self, binary: &BinaryRecord) -> Result<()> {
        let connection = self.connection()?;
        let is_custom = connection
            .query_row(
                "SELECT discovery_source FROM detected_binaries WHERE executable_path = ?1",
                [&binary.executable_path],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .is_some_and(|source| source == "Custom registration");
        drop(connection);
        if is_custom {
            Ok(())
        } else {
            self.save_binary(binary)
        }
    }

    pub fn remove_binary(&self, id: &str) -> Result<()> {
        let removed = self
            .connection()?
            .execute("DELETE FROM detected_binaries WHERE id = ?1", [id])?;
        if removed == 0 {
            return Err(anyhow!("binary registration no longer exists"));
        }
        Ok(())
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, name, path, runtime_binary_id, startup_arguments, detected_frameworks, created_at, environment_profile
             FROM projects ORDER BY name COLLATE NOCASE",
        )?;
        let rows = statement.query_map([], project_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn project(&self, id: &str) -> Result<Project> {
        self.connection()?
            .query_row(
                "SELECT id, name, path, runtime_binary_id, startup_arguments, detected_frameworks, created_at, environment_profile
                 FROM projects WHERE id = ?1",
                [id],
                project_from_row,
            )
            .optional()?
            .ok_or_else(|| anyhow!("project no longer exists"))
    }

    pub fn save_project(&self, project: &Project) -> Result<()> {
        self.connection()?.execute(
            "INSERT INTO projects (id, name, path, runtime_binary_id, startup_arguments, detected_frameworks, created_at, environment_profile)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(path) DO UPDATE SET
                name = excluded.name,
                runtime_binary_id = excluded.runtime_binary_id,
                startup_arguments = excluded.startup_arguments,
                detected_frameworks = excluded.detected_frameworks,
                environment_profile = excluded.environment_profile",
            params![
                project.id,
                project.name,
                project.path,
                project.runtime_binary_id,
                serde_json::to_string(&project.startup_arguments)?,
                serde_json::to_string(&project.detected_frameworks)?,
                project.created_at,
                serde_json::to_string(&project.environment_variables)?,
            ],
        )?;
        Ok(())
    }

    pub fn set_project_runtime(&self, project_id: &str, binary_id: Option<&str>) -> Result<()> {
        let connection = self.connection()?;
        if let Some(binary_id) = binary_id {
            let binary_exists: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM detected_binaries WHERE id = ?1)",
                [binary_id],
                |row| row.get(0),
            )?;
            if !binary_exists {
                return Err(anyhow!("runtime registration no longer exists"));
            }
        }
        let updated = connection.execute(
            "UPDATE projects SET runtime_binary_id = ?2 WHERE id = ?1",
            params![project_id, binary_id],
        )?;
        if updated == 0 {
            return Err(anyhow!("project no longer exists"));
        }
        Ok(())
    }

    pub fn update_project_configuration(
        &self,
        project_id: &str,
        binary_id: Option<&str>,
        startup_arguments: &[String],
        environment_variables: &[EnvironmentVariable],
    ) -> Result<()> {
        let connection = self.connection()?;
        if let Some(binary_id) = binary_id {
            let binary_exists: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM detected_binaries WHERE id = ?1)",
                [binary_id],
                |row| row.get(0),
            )?;
            if !binary_exists {
                return Err(anyhow!("runtime registration no longer exists"));
            }
        }
        let updated = connection.execute(
            "UPDATE projects SET runtime_binary_id = ?2, startup_arguments = ?3, environment_profile = ?4 WHERE id = ?1",
            params![
                project_id,
                binary_id,
                serde_json::to_string(startup_arguments)?,
                serde_json::to_string(environment_variables)?,
            ],
        )?;
        if updated == 0 {
            return Err(anyhow!("project no longer exists"));
        }
        Ok(())
    }

    pub fn record_process(&self, process: &ProcessSnapshot) -> Result<()> {
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO process_sessions (id, project_id, pid, started_at, status, exit_code, error_detail)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                format!(
                    "{}:{}:{}",
                    process.project_id, process.started_at, process.pid
                ),
                process.project_id,
                process.pid,
                process.started_at,
                status_name(&process.status),
                process.exit_code,
                process.error_detail,
            ],
        )?;
        Ok(())
    }

    pub fn finish_process(
        &self,
        project_id: &str,
        pid: u32,
        started_at: i64,
        status: &ProcessStatus,
        exit_code: Option<i32>,
        error_detail: Option<&str>,
    ) -> Result<()> {
        self.connection()?.execute(
            "UPDATE process_sessions SET status = ?4, exit_code = ?5, error_detail = ?6 WHERE project_id = ?1 AND pid = ?2 AND started_at = ?3",
            params![project_id, pid, started_at, status_name(status), exit_code, error_detail],
        )?;
        Ok(())
    }

    pub fn update_process_status(
        &self,
        project_id: &str,
        pid: u32,
        started_at: i64,
        status: &ProcessStatus,
    ) -> Result<()> {
        self.connection()?.execute(
            "UPDATE process_sessions SET status = ?4 WHERE project_id = ?1 AND pid = ?2 AND started_at = ?3",
            params![project_id, pid, started_at, status_name(status)],
        )?;
        Ok(())
    }

    pub fn reconcile_processes(&self) -> Result<()> {
        self.connection()?.execute(
            "UPDATE process_sessions SET status = 'unknown' WHERE status IN ('running', 'stopping')",
            [],
        )?;
        Ok(())
    }

    pub fn list_managed_services(&self) -> Result<Vec<ManagedServiceProfile>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, name, kind, executable_path, arguments, data_directory, ports, created_at
             FROM managed_services ORDER BY name COLLATE NOCASE",
        )?;
        let rows = statement.query_map([], |row| {
            let arguments: String = row.get(4)?;
            let ports: String = row.get(6)?;
            Ok(ManagedServiceProfile {
                id: row.get(0)?,
                name: row.get(1)?,
                kind: row.get(2)?,
                executable_path: row.get(3)?,
                arguments: serde_json::from_str(&arguments).unwrap_or_default(),
                data_directory: row.get(5)?,
                ports: serde_json::from_str(&ports).unwrap_or_default(),
                created_at: row.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn save_managed_service(&self, service: &ManagedServiceProfile) -> Result<()> {
        self.connection()?.execute(
            "INSERT INTO managed_services (id, name, kind, executable_path, arguments, data_directory, ports, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET name=excluded.name, kind=excluded.kind,
                executable_path=excluded.executable_path, arguments=excluded.arguments,
                data_directory=excluded.data_directory, ports=excluded.ports",
            params![
                service.id, service.name, service.kind, service.executable_path,
                serde_json::to_string(&service.arguments)?, service.data_directory,
                serde_json::to_string(&service.ports)?, service.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn remove_managed_service(&self, id: &str) -> Result<()> {
        let removed = self
            .connection()?
            .execute("DELETE FROM managed_services WHERE id=?1", [id])?;
        if removed == 0 {
            return Err(anyhow!("managed service profile no longer exists"));
        }
        Ok(())
    }

    pub fn create_workspace(&self, id: &str, name: &str, created_at: i64) -> Result<()> {
        self.connection()?
            .execute(
                "INSERT INTO workspaces (id, name, created_at) VALUES (?1, ?2, ?3)",
                params![id, name, created_at],
            )
            .map_err(|error| anyhow!("creating workspace: {error}"))?;
        Ok(())
    }

    pub fn list_workspaces(&self) -> Result<Vec<Workspace>> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare("SELECT id, name, created_at FROM workspaces ORDER BY name COLLATE NOCASE")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        let headers = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        headers.into_iter().map(|(id, name, created_at)| {
            let mut project_statement = connection.prepare("SELECT project_id FROM workspace_projects WHERE workspace_id=?1 ORDER BY position, project_id")?;
            let project_ids = project_statement.query_map([&id], |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            let mut dependency_statement = connection.prepare("SELECT project_id, depends_on_project_id FROM project_dependencies WHERE workspace_id=?1")?;
            let dependency_rows = dependency_statement.query_map([&id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
            let mut dependencies = std::collections::HashMap::<String, Vec<String>>::new();
            for dependency in dependency_rows {
                let (project, prerequisite) = dependency?;
                dependencies.entry(project).or_default().push(prerequisite);
            }
            let mut service_statement = connection.prepare("SELECT service_id FROM workspace_service_consumers WHERE workspace_id=?1")?;
            let service_ids = service_statement.query_map([&id], |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(Workspace { id, name, created_at, project_ids, dependencies, service_ids })
        }).collect()
    }

    pub fn add_workspace_project(&self, workspace_id: &str, project_id: &str) -> Result<()> {
        self.connection()?.execute(
            "INSERT OR IGNORE INTO workspace_projects (workspace_id, project_id) VALUES (?1, ?2)",
            params![workspace_id, project_id],
        ).map_err(|error| anyhow!("adding project to workspace: {error}"))?;
        Ok(())
    }

    pub fn set_project_dependency(
        &self,
        workspace_id: &str,
        project_id: &str,
        prerequisite_id: &str,
        depends: bool,
    ) -> Result<()> {
        let connection = self.connection()?;
        let statement = if depends {
            "INSERT OR IGNORE INTO project_dependencies (workspace_id, project_id, depends_on_project_id) VALUES (?1, ?2, ?3)"
        } else {
            "DELETE FROM project_dependencies WHERE workspace_id=?1 AND project_id=?2 AND depends_on_project_id=?3"
        };
        connection
            .execute(
                statement,
                params![workspace_id, project_id, prerequisite_id],
            )
            .map_err(|error| anyhow!("updating workspace dependency: {error}"))?;
        Ok(())
    }

    pub fn add_workspace_service(&self, workspace_id: &str, service_id: &str) -> Result<()> {
        self.connection()?.execute(
            "INSERT OR IGNORE INTO workspace_service_consumers (workspace_id, service_id) VALUES (?1, ?2)",
            params![workspace_id, service_id],
        ).map_err(|error| anyhow!("linking workspace service: {error}"))?;
        Ok(())
    }

    pub fn workspace_service_consumers(&self, service_id: &str) -> Result<usize> {
        let count: i64 = self.connection()?.query_row(
            "SELECT COUNT(*) FROM workspace_service_consumers WHERE service_id=?1",
            [service_id],
            |row| row.get(0),
        )?;
        Ok(count.max(0) as usize)
    }

    pub fn workspace_services(&self, workspace_id: &str) -> Result<Vec<String>> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare("SELECT service_id FROM workspace_service_consumers WHERE workspace_id=?1")?;
        let rows = statement.query_map([workspace_id], |row| row.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn list_local_domains(&self) -> Result<Vec<LocalDomain>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, project_id, domain, upstream, websocket FROM local_domains ORDER BY domain",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(LocalDomain {
                id: row.get(0)?,
                project_id: row.get(1)?,
                domain: row.get(2)?,
                upstream: row.get(3)?,
                websocket: row.get::<_, i64>(4)? != 0,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn save_local_domain(&self, domain: &LocalDomain) -> Result<()> {
        self.connection()?.execute(
            "INSERT INTO local_domains (id, project_id, domain, upstream, websocket) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET project_id=excluded.project_id, domain=excluded.domain, upstream=excluded.upstream, websocket=excluded.websocket",
            params![domain.id, domain.project_id, domain.domain, domain.upstream, domain.websocket],
        ).map_err(|error| anyhow!("saving local domain: {error}"))?;
        Ok(())
    }

    pub fn remove_local_domain(&self, id: &str) -> Result<()> {
        if self
            .connection()?
            .execute("DELETE FROM local_domains WHERE id=?1", [id])?
            == 0
        {
            return Err(anyhow!("local domain registration no longer exists"));
        }
        Ok(())
    }

    pub fn list_extensions(&self) -> Result<Vec<ExtensionRecord>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare("SELECT id, name, version, api_version, manifest_path, capabilities, approved_capabilities, enabled FROM extensions ORDER BY name COLLATE NOCASE")?;
        let rows = statement.query_map([], |row| {
            let capabilities: String = row.get(5)?;
            let approved: String = row.get(6)?;
            Ok(ExtensionRecord {
                id: row.get(0)?,
                name: row.get(1)?,
                version: row.get(2)?,
                api_version: row.get(3)?,
                manifest_path: row.get(4)?,
                capabilities: serde_json::from_str(&capabilities).unwrap_or_default(),
                approved_capabilities: serde_json::from_str(&approved).unwrap_or_default(),
                enabled: row.get::<_, i64>(7)? != 0,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn save_extension(&self, extension: &ExtensionRecord) -> Result<()> {
        self.connection()?.execute(
            "INSERT INTO extensions (id, name, version, api_version, manifest_path, capabilities, approved_capabilities, enabled)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(manifest_path) DO UPDATE SET name=excluded.name, version=excluded.version,
                api_version=excluded.api_version, capabilities=excluded.capabilities,
                approved_capabilities=excluded.approved_capabilities, enabled=excluded.enabled",
            params![extension.id, extension.name, extension.version, extension.api_version,
                extension.manifest_path, serde_json::to_string(&extension.capabilities)?,
                serde_json::to_string(&extension.approved_capabilities)?, extension.enabled],
        )?;
        Ok(())
    }

    pub fn set_extension_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        if self.connection()?.execute(
            "UPDATE extensions SET enabled=?2 WHERE id=?1",
            params![id, enabled],
        )? == 0
        {
            return Err(anyhow!("extension registration no longer exists"));
        }
        Ok(())
    }

    pub fn remove_extension(&self, id: &str) -> Result<()> {
        if self
            .connection()?
            .execute("DELETE FROM extensions WHERE id=?1", [id])?
            == 0
        {
            return Err(anyhow!("extension registration no longer exists"));
        }
        Ok(())
    }

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .connection()?
            .query_row(
                "SELECT value FROM application_settings WHERE key = ?1",
                [key],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.connection()?.execute(
            "INSERT INTO application_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

fn binary_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<BinaryRecord> {
    let category: String = row.get(2)?;
    let category = match category.as_str() {
        "Tool" => BinaryCategory::Tool,
        "Infrastructure" => BinaryCategory::Infrastructure,
        _ => BinaryCategory::Runtime,
    };
    let arguments: String = row.get(5)?;
    Ok(BinaryRecord {
        id: row.get(0)?,
        display_name: row.get(1)?,
        category,
        executable_path: row.get(3)?,
        resolved_path: row.get(4)?,
        version_arguments: serde_json::from_str(&arguments).unwrap_or_default(),
        version_parser: row.get(6)?,
        version_regex: row.get(7)?,
        detected_version: row.get(8)?,
        manual_version: row.get(9)?,
        discovery_source: row.get(10)?,
        trust_status: row.get(11)?,
        enabled: row.get::<_, i64>(12)? != 0,
        last_verified_at: row.get(13)?,
        probe_error: row.get(14)?,
    })
}

fn has_column(connection: &Connection, table: &str, column: &str) -> Result<bool> {
    // Table names are fixed migration constants at each call site.
    let query =
        format!("SELECT EXISTS(SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?1)");
    Ok(connection.query_row(&query, [column], |row| row.get(0))?)
}

fn project_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    let arguments: String = row.get(4)?;
    let frameworks: String = row.get(5)?;
    let environment: String = row.get(7)?;
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        path: row.get(2)?,
        runtime_binary_id: row.get(3)?,
        startup_arguments: serde_json::from_str(&arguments).unwrap_or_default(),
        environment_variables: serde_json::from_str(&environment).unwrap_or_default(),
        detected_frameworks: serde_json::from_str(&frameworks).unwrap_or_default(),
        created_at: row.get(6)?,
    })
}

fn status_name(status: &ProcessStatus) -> &'static str {
    match status {
        ProcessStatus::Running => "running",
        ProcessStatus::Stopping => "stopping",
        ProcessStatus::Exited => "exited",
        ProcessStatus::Failed => "failed",
    }
}
