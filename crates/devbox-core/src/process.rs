use crate::model::{ProcessSnapshot, ProcessStatus, Project, now_unix};
use crate::storage::Storage;
use anyhow::{Context, Result, anyhow};
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, BufReader};
use tokio::process::Command;

const MAX_LOG_LINES: usize = 500;
const METRICS_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Default)]
pub struct ProcessManager {
    processes: Arc<RwLock<HashMap<String, ProcessSnapshot>>>,
}

impl ProcessManager {
    pub async fn start(
        &self,
        storage: Storage,
        project: Project,
        executable: &Path,
    ) -> Result<ProcessSnapshot> {
        if !project.path.is_empty() && !Path::new(&project.path).is_dir() {
            return Err(anyhow!("project directory is no longer accessible"));
        }
        if self
            .processes
            .read()
            .map_err(|_| anyhow!("process state lock poisoned"))?
            .get(&project.id)
            .is_some_and(|process| {
                matches!(
                    process.status,
                    ProcessStatus::Running | ProcessStatus::Stopping
                )
            })
        {
            return Err(anyhow!("a process for this project is already running"));
        }

        let mut command = Command::new(executable);
        command
            .args(&project.startup_arguments)
            .envs(
                project
                    .environment_variables
                    .iter()
                    .map(|variable| (variable.key.as_str(), variable.value.as_str())),
            )
            .current_dir(&project.path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        crate::platform::configure_owned_process(&mut command);

        let mut child = command
            .spawn()
            .with_context(|| format!("starting {}", executable.display()))?;
        let pid = child
            .id()
            .ok_or_else(|| anyhow!("operating system did not return a process ID"))?;
        let started_at = now_unix();
        let snapshot = ProcessSnapshot {
            project_id: project.id.clone(),
            project_name: project.name.clone(),
            pid,
            started_at,
            status: ProcessStatus::Running,
            exit_code: None,
            error_detail: None,
            cpu_percent: None,
            memory_bytes: None,
            logs: VecDeque::new(),
        };
        self.processes
            .write()
            .map_err(|_| anyhow!("process state lock poisoned"))?
            .insert(project.id.clone(), snapshot.clone());
        if let Err(error) = storage.record_process(&snapshot) {
            let _ = child.start_kill();
            if let Ok(mut processes) = self.processes.write() {
                processes.remove(&project.id);
            }
            return Err(error);
        }

        let secret_values = project
            .environment_variables
            .iter()
            .filter(|variable| variable.secret)
            .map(|variable| variable.value.clone())
            .collect::<Vec<_>>();
        if let Some(stdout) = child.stdout.take() {
            self.spawn_log_reader(project.id.clone(), "stdout", stdout, secret_values.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            self.spawn_log_reader(project.id.clone(), "stderr", stderr, secret_values);
        }
        self.spawn_metrics_reader(project.id.clone(), pid);

        let states = Arc::clone(&self.processes);
        tokio::spawn(async move {
            let result = child.wait().await;
            // A launcher may exit while leaving descendants in its owned process group.
            // Reap that group before publishing a terminal state.
            let _ = crate::platform::stop_owned_process_group(
                pid,
                Duration::from_millis(300),
                "project child",
            )
            .await;
            if let Ok(mut processes) = states.write()
                && let Some(process) = processes.get_mut(&project.id)
            {
                let requested_stop = process.status == ProcessStatus::Stopping;
                match result {
                    Ok(status) if requested_stop => {
                        process.status = ProcessStatus::Exited;
                        process.exit_code = status.code();
                    }
                    Ok(status) if status.success() => {
                        process.status = ProcessStatus::Exited;
                        process.exit_code = status.code();
                    }
                    Ok(status) => {
                        process.status = ProcessStatus::Failed;
                        process.exit_code = status.code();
                        process.error_detail = Some(format!("process exited with {status}"));
                    }
                    Err(error) => {
                        process.status = ProcessStatus::Failed;
                        process.error_detail =
                            Some(format!("failed while waiting for process: {error}"));
                        push_log(
                            process,
                            format!(
                                "[devbox] {}",
                                process.error_detail.as_deref().unwrap_or_default()
                            ),
                        );
                    }
                }
                let _ = storage.finish_process(
                    &process.project_id,
                    process.pid,
                    process.started_at,
                    &process.status,
                    process.exit_code,
                    process.error_detail.as_deref(),
                );
            }
        });

        Ok(snapshot)
    }

    pub async fn stop(&self, storage: Storage, project_id: &str) -> Result<()> {
        let (pid, started_at) = {
            let mut processes = self
                .processes
                .write()
                .map_err(|_| anyhow!("process state lock poisoned"))?;
            let process = processes
                .get_mut(project_id)
                .ok_or_else(|| anyhow!("no process is recorded for this project"))?;
            if !matches!(
                process.status,
                ProcessStatus::Running | ProcessStatus::Stopping
            ) {
                return Err(anyhow!("the project process is not running"));
            }
            process.status = ProcessStatus::Stopping;
            (process.pid, process.started_at)
        };

        if let Err(error) =
            storage.update_process_status(project_id, pid, started_at, &ProcessStatus::Stopping)
        {
            tracing::warn!(%error, project_id, "could not persist process stopping state");
        }

        let shutdown_result =
            crate::platform::stop_owned_process_group(pid, Duration::from_secs(3), "project").await;

        if let Err(error) = shutdown_result {
            if let Ok(mut processes) = self.processes.write()
                && let Some(process) = processes.get_mut(project_id)
                && process.started_at == started_at
                && process.status == ProcessStatus::Stopping
            {
                process.status = ProcessStatus::Running;
            }
            if let Err(persist_error) =
                storage.update_process_status(project_id, pid, started_at, &ProcessStatus::Running)
            {
                tracing::warn!(%persist_error, project_id, "could not restore process state after failed shutdown");
            }
            return Err(error);
        }
        Ok(())
    }

    pub async fn restart(
        &self,
        storage: Storage,
        project: Project,
        executable: &Path,
    ) -> Result<ProcessSnapshot> {
        let active = self
            .processes
            .read()
            .map_err(|_| anyhow!("process state lock poisoned"))?
            .get(&project.id)
            .is_some_and(|process| {
                matches!(
                    process.status,
                    ProcessStatus::Running | ProcessStatus::Stopping
                )
            });

        if active {
            self.stop(storage.clone(), &project.id).await?;
            let mut stopped = false;
            for _ in 0..50 {
                let active = self
                    .processes
                    .read()
                    .map_err(|_| anyhow!("process state lock poisoned"))?
                    .get(&project.id)
                    .is_some_and(|process| {
                        matches!(
                            process.status,
                            ProcessStatus::Running | ProcessStatus::Stopping
                        )
                    });
                if !active {
                    stopped = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            if !stopped {
                return Err(anyhow!("the existing process did not finish shutting down"));
            }
        }

        self.start(storage, project, executable).await
    }

    pub fn snapshots(&self) -> Vec<ProcessSnapshot> {
        self.processes
            .read()
            .map(|processes| {
                let mut snapshots = processes.values().cloned().collect::<Vec<_>>();
                snapshots.sort_by_key(|process| std::cmp::Reverse(process.started_at));
                snapshots
            })
            .unwrap_or_default()
    }

    fn spawn_log_reader<R>(
        &self,
        project_id: String,
        label: &'static str,
        reader: R,
        secret_values: Vec<String>,
    ) where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
    {
        let states = Arc::clone(&self.processes);
        tokio::spawn(async move {
            let mut reader = BufReader::new(reader);
            let mut chunk = [0u8; 4096];
            let mut line = Vec::with_capacity(1024);
            let mut truncated = false;
            loop {
                match reader.read(&mut chunk).await {
                    Ok(0) => break,
                    Ok(count) => {
                        for byte in &chunk[..count] {
                            if *byte == b'\n' {
                                emit_log_line(
                                    &states,
                                    &project_id,
                                    label,
                                    &line,
                                    truncated,
                                    &secret_values,
                                );
                                line.clear();
                                truncated = false;
                            } else if line.len() < 8192 {
                                line.push(*byte);
                            } else {
                                truncated = true;
                            }
                        }
                    }
                    Err(error) => {
                        push_process_log(
                            &states,
                            &project_id,
                            format!("[devbox] log capture ended: {error}"),
                        );
                        break;
                    }
                }
            }
            if !line.is_empty() || truncated {
                emit_log_line(
                    &states,
                    &project_id,
                    label,
                    &line,
                    truncated,
                    &secret_values,
                );
            }
        });
    }

    fn spawn_metrics_reader(&self, project_id: String, pid: u32) {
        let states = Arc::clone(&self.processes);
        tokio::spawn(async move {
            let mut previous_sample = None::<(u64, Instant)>;
            loop {
                tokio::time::sleep(METRICS_INTERVAL).await;
                let sample =
                    tokio::task::spawn_blocking(move || crate::platform::read_process_metrics(pid))
                        .await
                        .ok()
                        .and_then(Result::ok);
                let Some(sample) = sample else {
                    break;
                };
                let sampled_at = Instant::now();
                let cpu_percent = previous_sample
                    .as_ref()
                    .and_then(|(previous_ticks, previous_at)| {
                        let elapsed = sampled_at.duration_since(*previous_at).as_secs_f64();
                        let ticks_per_second = sample.ticks_per_second as f64;
                        (elapsed > 0.0 && ticks_per_second > 0.0).then(|| {
                            sample.cpu_ticks.saturating_sub(*previous_ticks) as f64
                                / ticks_per_second
                                / elapsed
                                * 100.0
                        })
                    })
                    .map(|value| value.min(f64::from(f32::MAX)) as f32);
                previous_sample = Some((sample.cpu_ticks, sampled_at));

                let should_continue = if let Ok(mut processes) = states.write()
                    && let Some(process) = processes.get_mut(&project_id)
                    && process.pid == pid
                    && matches!(
                        process.status,
                        ProcessStatus::Running | ProcessStatus::Stopping
                    ) {
                    process.cpu_percent = cpu_percent;
                    process.memory_bytes = Some(sample.memory_bytes);
                    true
                } else {
                    false
                };
                if !should_continue {
                    break;
                }
            }
        });
    }
}

fn emit_log_line(
    states: &Arc<RwLock<HashMap<String, ProcessSnapshot>>>,
    project_id: &str,
    label: &str,
    bytes: &[u8],
    truncated: bool,
    secret_values: &[String],
) {
    let mut line = String::from_utf8_lossy(bytes)
        .trim_end_matches('\r')
        .to_string();
    for secret in secret_values.iter().filter(|secret| !secret.is_empty()) {
        line = line.replace(secret, "[REDACTED]");
    }
    if truncated {
        line.push_str(" …[line truncated]");
    }
    push_process_log(states, project_id, format!("[{label}] {line}"));
}

fn push_process_log(
    states: &Arc<RwLock<HashMap<String, ProcessSnapshot>>>,
    project_id: &str,
    line: String,
) {
    if let Ok(mut processes) = states.write()
        && let Some(process) = processes.get_mut(project_id)
    {
        push_log(process, line);
    }
}

fn push_log(process: &mut ProcessSnapshot, line: String) {
    process.logs.push_back(line);
    while process.logs.len() > MAX_LOG_LINES {
        process.logs.pop_front();
    }
}
