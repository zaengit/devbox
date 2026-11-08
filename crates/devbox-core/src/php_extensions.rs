use anyhow::{Context, Result, anyhow};
use std::path::Path;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CAPTURE_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhpExtensionReport {
    pub loaded_extensions: Vec<String>,
    pub loaded_configuration: Option<String>,
    pub additional_ini_files: Vec<String>,
}

pub async fn inspect_php_extensions(executable: &Path) -> Result<PhpExtensionReport> {
    let modules_output = run_php_command(executable, &["-m"]).await?;
    let configuration_output = run_php_command(executable, &["--ini"]).await?;
    let loaded_extensions = modules_output
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line.starts_with('[')
                && !line.starts_with("Warning:")
                && !line.starts_with("Notice:")
        })
        .map(str::to_string)
        .collect();
    let loaded_configuration = labeled_value(&configuration_output, "Loaded Configuration File:")
        .filter(|value| value != "(none)");
    let additional_ini_files =
        labeled_value(&configuration_output, "Additional .ini files parsed:")
            .filter(|value| value != "(none)")
            .map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|path| !path.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
    Ok(PhpExtensionReport {
        loaded_extensions,
        loaded_configuration,
        additional_ini_files,
    })
}

async fn run_php_command(executable: &Path, arguments: &[&str]) -> Result<String> {
    let run = async {
        let mut child = Command::new(executable)
            .args(arguments)
            .kill_on_drop(true)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .with_context(|| format!("starting PHP inspection command {}", executable.display()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("PHP stdout was not captured"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("PHP stderr was not captured"))?;
        let read_stdout = async move {
            let mut bytes = Vec::new();
            stdout
                .take(MAX_CAPTURE_BYTES)
                .read_to_end(&mut bytes)
                .await?;
            Ok::<_, std::io::Error>(bytes)
        };
        let read_stderr = async move {
            let mut bytes = Vec::new();
            stderr
                .take(MAX_CAPTURE_BYTES)
                .read_to_end(&mut bytes)
                .await?;
            Ok::<_, std::io::Error>(bytes)
        };
        let (stdout, stderr, status) = tokio::try_join!(read_stdout, read_stderr, child.wait())
            .context("reading PHP inspection output")?;
        let stdout = String::from_utf8_lossy(&stdout).to_string();
        let stderr = String::from_utf8_lossy(&stderr).to_string();
        if !status.success() {
            return Err(anyhow!(
                "PHP inspection command {:?} exited with {status}: {}",
                arguments,
                first_line(&stderr)
                    .or_else(|| first_line(&stdout))
                    .unwrap_or_default()
            ));
        }
        Ok::<_, anyhow::Error>(format!("{stdout}\n{stderr}"))
    };

    match tokio::time::timeout(COMMAND_TIMEOUT, run).await {
        Ok(result) => result,
        Err(_) => Err(anyhow!(
            "PHP inspection command {:?} exceeded {} seconds",
            arguments,
            COMMAND_TIMEOUT.as_secs()
        )),
    }
}

fn labeled_value(output: &str, label: &str) -> Option<String> {
    output.lines().find_map(|line| {
        line.trim()
            .strip_prefix(label)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn first_line(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| {
            line.chars()
                .filter(|character| !character.is_control())
                .take(240)
                .collect()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_loaded_extensions_and_php_ini_paths() {
        let modules = "[PHP Modules]\nCore\nPDO\nredis\n\n[Zend Modules]\nZend OPcache\n";
        let config = "Configuration File (php.ini) Path: /etc/php\nLoaded Configuration File: /opt/php/php.ini\nAdditional .ini files parsed: /etc/php/conf.d/redis.ini, /etc/php/conf.d/xdebug.ini\n";
        let report = PhpExtensionReport {
            loaded_extensions: modules
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('['))
                .map(str::to_string)
                .collect(),
            loaded_configuration: labeled_value(config, "Loaded Configuration File:"),
            additional_ini_files: labeled_value(config, "Additional .ini files parsed:")
                .unwrap()
                .split(',')
                .map(str::trim)
                .map(str::to_string)
                .collect(),
        };
        assert_eq!(
            report.loaded_extensions,
            ["Core", "PDO", "redis", "Zend OPcache"]
        );
        assert_eq!(
            report.loaded_configuration.as_deref(),
            Some("/opt/php/php.ini")
        );
        assert_eq!(report.additional_ini_files.len(), 2);
    }
}
