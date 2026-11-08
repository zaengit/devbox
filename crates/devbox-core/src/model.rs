use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum BinaryCategory {
    Runtime,
    Tool,
    Infrastructure,
}

impl BinaryCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Runtime => "Runtime",
            Self::Tool => "Tool",
            Self::Infrastructure => "Infrastructure",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BinaryRecord {
    pub id: String,
    pub display_name: String,
    pub category: BinaryCategory,
    pub executable_path: String,
    pub resolved_path: Option<String>,
    pub version_arguments: Vec<String>,
    pub version_parser: String,
    pub version_regex: Option<String>,
    pub detected_version: Option<String>,
    /// Last diagnostic from a failed version probe. Kept separately from the
    /// availability label so users can understand why a discovered file failed.
    pub probe_error: Option<String>,
    pub manual_version: Option<String>,
    pub discovery_source: String,
    pub trust_status: String,
    pub enabled: bool,
    pub last_verified_at: Option<i64>,
}

impl BinaryRecord {
    pub fn version(&self) -> Option<&str> {
        self.manual_version
            .as_deref()
            .or(self.detected_version.as_deref())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
    pub runtime_binary_id: Option<String>,
    pub startup_arguments: Vec<String>,
    pub environment_variables: Vec<EnvironmentVariable>,
    pub detected_frameworks: Vec<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EnvironmentVariable {
    pub key: String,
    pub value: String,
    pub secret: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ManagedServiceProfile {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub executable_path: String,
    pub arguments: Vec<String>,
    pub data_directory: Option<String>,
    pub ports: Vec<u16>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub project_ids: Vec<String>,
    /// Each key is a project ID and each value is its direct prerequisites.
    pub dependencies: std::collections::HashMap<String, Vec<String>>,
    pub service_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ExtensionRecord {
    pub id: String,
    pub name: String,
    pub version: String,
    pub api_version: u32,
    pub manifest_path: String,
    pub capabilities: Vec<String>,
    pub approved_capabilities: Vec<String>,
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct LocalDomain {
    pub id: String,
    pub project_id: String,
    pub domain: String,
    pub upstream: String,
    pub websocket: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProcessSnapshot {
    pub project_id: String,
    pub project_name: String,
    pub pid: u32,
    pub started_at: i64,
    pub status: ProcessStatus,
    pub exit_code: Option<i32>,
    pub error_detail: Option<String>,
    pub cpu_percent: Option<f32>,
    pub memory_bytes: Option<u64>,
    pub logs: VecDeque<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ProcessStatus {
    Running,
    Stopping,
    Exited,
    Failed,
}

pub(crate) fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
