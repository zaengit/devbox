use crate::model::{BinaryCategory, BinaryRecord, now_unix};
use anyhow::{Context, Result, anyhow};
use futures::{StreamExt, stream};
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

const PROBE_TIMEOUT: Duration = Duration::from_secs(4);
const MAX_CAPTURE_BYTES: u64 = 16 * 1024;
const MAX_CUSTOM_SEARCH_DIRECTORIES: usize = 64;

#[derive(Clone, Debug)]
pub struct ProbeResult {
    pub version: Option<String>,
    pub error: Option<String>,
}

const TOOLS: &[(&str, &str, BinaryCategory, &[&str])] = &[
    ("php", "PHP", BinaryCategory::Runtime, &["--version"]),
    ("go", "Go", BinaryCategory::Runtime, &["version"]),
    ("rustc", "Rust", BinaryCategory::Runtime, &["--version"]),
    ("cargo", "Cargo", BinaryCategory::Tool, &["--version"]),
    ("node", "Node.js", BinaryCategory::Runtime, &["--version"]),
    ("npm", "npm", BinaryCategory::Tool, &["--version"]),
    ("pnpm", "pnpm", BinaryCategory::Tool, &["--version"]),
    ("python", "Python", BinaryCategory::Runtime, &["--version"]),
    (
        "python3",
        "Python 3",
        BinaryCategory::Runtime,
        &["--version"],
    ),
    ("ruby", "Ruby", BinaryCategory::Runtime, &["--version"]),
    ("composer", "Composer", BinaryCategory::Tool, &["--version"]),
];

pub fn validate_executable(path: &Path) -> Result<PathBuf> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("cannot access executable {}", path.display()))?;
    if !metadata.is_file() {
        return Err(anyhow!("selected path is not a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(anyhow!("file does not have executable permission"));
        }
    }
    path.canonicalize()
        .with_context(|| format!("resolving executable path {}", path.display()))
}

pub async fn test_binary(path: &Path, args: &[String], version_regex: Option<&str>) -> ProbeResult {
    match validate_executable(path) {
        Ok(resolved) => probe_binary(&resolved, args, version_regex).await,
        Err(error) => ProbeResult {
            version: None,
            error: Some(error.to_string()),
        },
    }
}

pub async fn probe_binary(
    path: &Path,
    args: &[String],
    version_regex: Option<&str>,
) -> ProbeResult {
    let result = async {
        let mut child = Command::new(path)
            .args(args)
            .kill_on_drop(true)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .with_context(|| format!("starting {}", path.display()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("stdout was not captured"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("stderr was not captured"))?;
        let read_stdout = async move {
            let mut bytes = Vec::new();
            stdout
                .take(MAX_CAPTURE_BYTES)
                .read_to_end(&mut bytes)
                .await?;
            Ok::<_, io::Error>(bytes)
        };
        let read_stderr = async move {
            let mut bytes = Vec::new();
            stderr
                .take(MAX_CAPTURE_BYTES)
                .read_to_end(&mut bytes)
                .await?;
            Ok::<_, io::Error>(bytes)
        };
        let (stdout, stderr, status) = tokio::try_join!(read_stdout, read_stderr, child.wait())
            .context("reading version command output")?;
        let output = format!(
            "{}\n{}",
            String::from_utf8_lossy(&stdout),
            String::from_utf8_lossy(&stderr)
        );
        if !status.success() {
            return Err(anyhow!(
                "version command exited with {status}: {}",
                first_line(&output)
            ));
        }
        let version = extract_version(&output, version_regex)?.ok_or_else(|| {
            let excerpt = first_line(&output);
            if excerpt.is_empty() {
                anyhow!("version command produced no readable version output")
            } else {
                anyhow!("version command output did not contain a recognizable version: {excerpt}")
            }
        })?;
        Ok::<_, anyhow::Error>(version)
    };

    match tokio::time::timeout(PROBE_TIMEOUT, result).await {
        Ok(Ok(version)) => ProbeResult {
            version: Some(version),
            error: None,
        },
        Ok(Err(error)) => ProbeResult {
            version: None,
            error: Some(error.to_string()),
        },
        Err(_) => ProbeResult {
            version: None,
            error: Some(format!(
                "version check exceeded {} seconds",
                PROBE_TIMEOUT.as_secs()
            )),
        },
    }
}

pub async fn scan_path() -> Vec<BinaryRecord> {
    scan_path_with_directories(Vec::new()).await
}

pub async fn scan_path_with_directories(custom_directories: Vec<PathBuf>) -> Vec<BinaryRecord> {
    let path_directories = std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default();
    let mut known_directories = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        known_directories.extend([
            home.join(".local/bin"),
            home.join(".cargo/bin"),
            home.join(".local/share/mise/shims"),
            home.join(".nvm/current/bin"),
        ]);
    }
    known_directories.extend([
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
    ]);

    scan_directories(path_directories, known_directories, custom_directories).await
}

async fn scan_directories(
    path_directories: Vec<PathBuf>,
    known_directories: Vec<PathBuf>,
    custom_directories: Vec<PathBuf>,
) -> Vec<BinaryRecord> {
    let path_set = path_directories.iter().cloned().collect::<HashSet<_>>();
    let custom_set = custom_directories.iter().cloned().collect::<HashSet<_>>();
    let directories = path_directories
        .into_iter()
        .chain(known_directories)
        .chain(
            custom_directories
                .into_iter()
                .take(MAX_CUSTOM_SEARCH_DIRECTORIES),
        )
        .collect::<Vec<_>>();
    let mut found =
        HashMap::<PathBuf, (String, String, BinaryCategory, Vec<String>, String)>::new();
    for directory in directories {
        let source = if path_set.contains(&directory) {
            "PATH"
        } else if custom_set.contains(&directory) {
            "Custom search directory"
        } else {
            "Known directory"
        };
        for (command, label, category, args) in TOOLS {
            let candidate = directory.join(command);
            if !candidate.exists() {
                continue;
            }
            let Ok(resolved) = candidate.canonicalize() else {
                continue;
            };
            if !resolved.is_file() {
                continue;
            }
            found.entry(resolved).or_insert_with(|| {
                (
                    (*command).to_string(),
                    (*label).to_string(),
                    category.clone(),
                    args.iter()
                        .map(|argument| (*argument).to_string())
                        .collect(),
                    source.to_string(),
                )
            });
        }
    }

    stream::iter(found.into_iter().map(
        |(path, (command, label, category, arguments, source))| async move {
            let path_text = path.display().to_string();
            let probe = probe_binary(&path, &arguments, None).await;
            let trust_status = if probe.version.is_some() {
                "Available"
            } else {
                "Probe failed"
            };
            BinaryRecord {
                id: stable_path_id(&path_text),
                display_name: label,
                category,
                executable_path: path_text.clone(),
                resolved_path: Some(path_text),
                version_arguments: arguments,
                version_parser: "auto".to_string(),
                version_regex: Some(
                    r"(?i)(?:v|go)?([0-9]+(?:\.[0-9]+)+(?:[-+][A-Za-z0-9.-]+)?)".to_string(),
                ),
                detected_version: probe.version,
                probe_error: probe.error,
                manual_version: None,
                discovery_source: format!("{source} ({command})"),
                trust_status: trust_status.to_string(),
                enabled: true,
                last_verified_at: Some(now_unix()),
            }
        },
    ))
    .buffer_unordered(4)
    .collect::<Vec<_>>()
    .await
}

pub fn parse_arguments(input: &str) -> Vec<String> {
    input.split_whitespace().map(str::to_string).collect()
}

pub fn extract_version(output: &str, pattern: Option<&str>) -> Result<Option<String>> {
    if let Some(pattern) = pattern {
        let regex = Regex::new(pattern).context("invalid version regular expression")?;
        if let Some(captures) = regex.captures(output) {
            let version = captures
                .get(1)
                .or_else(|| captures.get(0))
                .map(|value| value.as_str().trim());
            return Ok(version
                .filter(|value| !value.is_empty())
                .map(str::to_string));
        }
        return Ok(None);
    }
    let line = first_line(output);
    let version_regex = Regex::new(r"(?i)(?:v|go)?([0-9]+(?:\.[0-9]+)+(?:[-+][A-Za-z0-9.-]+)?)")?;
    Ok(version_regex
        .captures(&line)
        .and_then(|captures| captures.get(1).or_else(|| captures.get(0)))
        .map(|value| value.as_str().to_string()))
}

fn first_line(output: &str) -> String {
    output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .chars()
        .filter(|character| !character.is_control())
        .take(240)
        .collect()
}

fn stable_path_id(path: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in path.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("detected-{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    fn executable(directory: &Path, name: &str, output: &str) -> PathBuf {
        fs::create_dir_all(directory).unwrap();
        let path = directory.join(name);
        fs::write(&path, format!("#!/bin/sh\n{output}\n")).unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }

    #[tokio::test]
    async fn scans_each_path_directory_without_recursion_and_resolves_symlinks() {
        let temp = TempDir::new().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        executable(&first, "node", "echo 'v20.11.1'");
        executable(&second, "node", "echo 'v22.1.0'");
        let nested = first.join("nested");
        executable(&nested, "node", "echo 'v99.0.0'");
        let alias = temp.path().join("alias");
        fs::create_dir(&alias).unwrap();
        std::os::unix::fs::symlink(first.join("node"), alias.join("node")).unwrap();

        let binaries = scan_directories(vec![first, second, alias], Vec::new(), Vec::new()).await;
        let node_versions = binaries
            .iter()
            .filter(|binary| binary.display_name == "Node.js")
            .map(|binary| binary.version().unwrap().to_string())
            .collect::<HashSet<_>>();
        assert_eq!(
            node_versions,
            HashSet::from(["20.11.1".into(), "22.1.0".into()])
        );
        assert_eq!(binaries.len(), 2, "nested executables are not scanned");
        assert!(
            binaries
                .iter()
                .all(|binary| binary.discovery_source.starts_with("PATH (node)"))
        );
        assert!(
            binaries
                .iter()
                .all(|binary| binary.id.starts_with("detected-"))
        );
    }

    #[tokio::test]
    async fn scans_configured_directories_without_recursion_and_marks_the_source() {
        let temp = TempDir::new().unwrap();
        let custom = temp.path().join("custom-tools");
        executable(&custom, "pnpm", "echo '12.16.1'");
        let binaries = scan_directories(Vec::new(), Vec::new(), vec![custom]).await;
        let pnpm = binaries
            .iter()
            .find(|binary| {
                binary.display_name == "pnpm"
                    && binary
                        .discovery_source
                        .starts_with("Custom search directory")
            })
            .unwrap();
        assert_eq!(pnpm.version(), Some("12.16.1"));
        assert!(pnpm.discovery_source.starts_with("Custom search directory"));
    }

    #[tokio::test]
    async fn probes_stderr_and_reports_failed_exit_or_invalid_parser() {
        let temp = TempDir::new().unwrap();
        let stderr = temp.path().join("stderr-runtime");
        fs::write(&stderr, "#!/bin/sh\necho 'Runtime 3.2.1' >&2\n").unwrap();
        let mut permissions = fs::metadata(&stderr).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&stderr, permissions).unwrap();
        assert_eq!(
            test_binary(&stderr, &[], None).await.version.as_deref(),
            Some("3.2.1")
        );

        let failure = executable(temp.path(), "failure", "echo 'Runtime 3.2.1'\nexit 7");
        let result = test_binary(&failure, &[], None).await;
        assert!(result.version.is_none());
        assert!(result.error.unwrap().contains("exited with"));

        let invalid_regex = test_binary(&stderr, &[], Some("[")).await;
        assert!(
            invalid_regex
                .error
                .unwrap()
                .contains("invalid version regular expression")
        );
    }

    #[tokio::test]
    async fn stops_a_hung_version_command_at_the_probe_timeout() {
        let temp = TempDir::new().unwrap();
        let hanging = executable(temp.path(), "hanging", "exec sleep 10");
        let started = tokio::time::Instant::now();
        let result = test_binary(&hanging, &[], None).await;
        assert!(result.version.is_none());
        assert!(result.error.unwrap().contains("exceeded 4 seconds"));
        assert!(started.elapsed() >= PROBE_TIMEOUT);
    }

    #[test]
    fn stable_ids_are_repeatable_and_path_specific() {
        assert_eq!(
            stable_path_id("/usr/bin/node"),
            stable_path_id("/usr/bin/node")
        );
        assert_ne!(
            stable_path_id("/usr/bin/node"),
            stable_path_id("/opt/node/bin/node")
        );
    }
}
