//! Narrow operating-system adapter surface for child process ownership and metrics.
//! Platform-specific implementations can be added here without changing orchestration logic.
use anyhow::{Context, Result, anyhow};
use std::time::Duration;
use tokio::process::Command;

#[derive(Clone, Copy, Debug)]
pub(crate) struct MetricsSample {
    pub cpu_ticks: u64,
    pub ticks_per_second: u64,
    pub memory_bytes: u64,
}

pub(crate) fn configure_owned_process(command: &mut Command) {
    #[cfg(unix)]
    command.process_group(0);
}

pub(crate) async fn stop_owned_process_group(
    pid: u32,
    grace: Duration,
    target: &str,
) -> Result<()> {
    #[cfg(unix)]
    {
        use nix::sys::signal::{Signal, killpg};
        use nix::unistd::Pid;
        let group = Pid::from_raw(pid as i32);
        if let Err(error) = killpg(group, Some(Signal::SIGTERM))
            && error != nix::errno::Errno::ESRCH
        {
            return Err(anyhow!("sending graceful {target} shutdown: {error}"));
        }
        let deadline = tokio::time::Instant::now() + grace;
        while tokio::time::Instant::now() < deadline {
            if !process_group_exists(group)? {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if process_group_exists(group)?
            && let Err(error) = killpg(group, Some(Signal::SIGKILL))
            && error != nix::errno::Errno::ESRCH
        {
            return Err(anyhow!("forcing {target} process group shutdown: {error}"));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (pid, grace, target);
        Err(anyhow!(
            "safe process-group shutdown is not implemented on this platform"
        ))
    }
}

#[cfg(unix)]
fn process_group_exists(group: nix::unistd::Pid) -> Result<bool> {
    use nix::sys::signal::killpg;
    match killpg(group, None) {
        Ok(()) => Ok(true),
        Err(nix::errno::Errno::ESRCH) => Ok(false),
        Err(nix::errno::Errno::EPERM) => Ok(true),
        Err(error) => Err(anyhow!("checking owned process group state: {error}")),
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn read_process_metrics(pid: u32) -> Result<MetricsSample> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .with_context(|| format!("reading process {pid} CPU usage"))?;
    let remainder = stat
        .rsplit_once(')')
        .map(|(_, remainder)| remainder)
        .ok_or_else(|| anyhow!("invalid /proc/{pid}/stat contents"))?;
    let fields = remainder.split_whitespace().collect::<Vec<_>>();
    let user_ticks = fields
        .get(11)
        .ok_or_else(|| anyhow!("missing user CPU ticks for process {pid}"))?
        .parse::<u64>()?;
    let system_ticks = fields
        .get(12)
        .ok_or_else(|| anyhow!("missing system CPU ticks for process {pid}"))?
        .parse::<u64>()?;
    let statm = std::fs::read_to_string(format!("/proc/{pid}/statm"))
        .with_context(|| format!("reading process {pid} memory usage"))?;
    let resident_pages = statm
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| anyhow!("missing resident memory for process {pid}"))?
        .parse::<u64>()?;
    // SAFETY: sysconf reads process-wide constants and has no pointer arguments.
    let ticks_per_second = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    // SAFETY: sysconf reads process-wide constants and has no pointer arguments.
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if ticks_per_second <= 0 || page_size <= 0 {
        return Err(anyhow!("could not read Linux process metric units"));
    }
    Ok(MetricsSample {
        cpu_ticks: user_ticks.saturating_add(system_ticks),
        ticks_per_second: ticks_per_second as u64,
        memory_bytes: resident_pages.saturating_mul(page_size as u64),
    })
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn read_process_metrics(_pid: u32) -> Result<MetricsSample> {
    Err(anyhow!(
        "process metrics are not supported on this platform"
    ))
}
