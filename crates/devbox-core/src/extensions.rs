use crate::ExtensionRecord;
use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use std::path::Path;

pub const EXTENSION_API_VERSION: u32 = 1;
pub const SUPPORTED_CAPABILITIES: &[&str] = &[
    "projects.read",
    "processes.read",
    "processes.start",
    "services.read",
    "services.manage",
    "storage.write",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    id: String,
    name: String,
    version: String,
    api_version: u32,
    capabilities: Vec<String>,
}

pub fn read_manifest(path: &Path) -> Result<ExtensionRecord> {
    let path = path
        .canonicalize()
        .with_context(|| format!("resolving extension manifest {}", path.display()))?;
    let metadata = std::fs::metadata(&path)?;
    if !metadata.is_file() || metadata.len() > 64 * 1024 {
        return Err(anyhow!(
            "extension manifest must be a regular file no larger than 64 KiB"
        ));
    }
    let manifest: Manifest = serde_json::from_slice(&std::fs::read(&path)?)
        .context("parsing extension manifest JSON")?;
    if manifest.api_version != EXTENSION_API_VERSION {
        return Err(anyhow!(
            "extension requires API {}, but Devbox supports API {}",
            manifest.api_version,
            EXTENSION_API_VERSION
        ));
    }
    if manifest.id.is_empty()
        || manifest.id.len() > 80
        || !manifest.id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'.'
        })
    {
        return Err(anyhow!(
            "extension id must use lowercase letters, digits, dots, or hyphens (up to 80 characters)"
        ));
    }
    if manifest.name.trim().is_empty()
        || manifest.name.len() > 120
        || manifest.version.trim().is_empty()
        || manifest.version.len() > 80
    {
        return Err(anyhow!(
            "extension name and version must be present and within their length limits"
        ));
    }
    if manifest.capabilities.len() > SUPPORTED_CAPABILITIES.len() {
        return Err(anyhow!("extension requests too many capabilities"));
    }
    let mut capabilities = manifest.capabilities;
    capabilities.sort();
    capabilities.dedup();
    for capability in &capabilities {
        if !SUPPORTED_CAPABILITIES.contains(&capability.as_str()) {
            return Err(anyhow!("unsupported extension capability: {capability}"));
        }
    }
    Ok(ExtensionRecord {
        id: manifest.id,
        name: manifest.name,
        version: manifest.version,
        api_version: manifest.api_version,
        manifest_path: path.display().to_string(),
        capabilities,
        approved_capabilities: Vec::new(),
        enabled: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn validates_supported_manifest_and_rejects_unknown_capabilities() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("devbox-extension.json");
        fs::write(&path, r#"{"id":"sample.tool","name":"Sample","version":"1.0.0","api_version":1,"capabilities":["projects.read"]}"#).unwrap();
        assert_eq!(
            read_manifest(&path).unwrap().capabilities,
            ["projects.read"]
        );
        fs::write(&path, r#"{"id":"sample.tool","name":"Sample","version":"1.0.0","api_version":1,"capabilities":["shell.execute"]}"#).unwrap();
        assert!(
            read_manifest(&path)
                .unwrap_err()
                .to_string()
                .contains("unsupported extension capability")
        );
        fs::write(&path, r#"{"id":"sample.tool","name":"Sample","version":"1.0.0","api_version":99,"capabilities":[]}"#).unwrap();
        assert!(
            read_manifest(&path)
                .unwrap_err()
                .to_string()
                .contains("supports API")
        );
    }
}
