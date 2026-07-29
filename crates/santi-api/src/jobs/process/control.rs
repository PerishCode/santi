use std::{
    path::Path,
    process::{Child, ExitStatus},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

use crate::jobs::{TIMEOUT, files};

const POLL: Duration = Duration::from_millis(20);

#[derive(Clone, Copy)]
pub(super) enum Halt {
    Output,
    Cancel,
    Timeout,
}

pub(super) fn wait(
    child: &mut Child,
    directory: &Path,
    exceeded: &AtomicBool,
) -> Result<(ExitStatus, Option<Halt>), String> {
    let deadline = timeout()?.map(|timeout| Instant::now() + timeout);
    loop {
        let halt = if exceeded.load(Ordering::Acquire) {
            Some(Halt::Output)
        } else if directory.join(files::CANCEL).is_file() {
            Some(Halt::Cancel)
        } else if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            Some(Halt::Timeout)
        } else {
            None
        };
        if let Some(halt) = halt {
            terminate(child)?;
            return child
                .wait()
                .map(|status| (status, Some(halt)))
                .map_err(|error| error.to_string());
        }
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            return Ok((status, None));
        }
        thread::sleep(POLL);
    }
}

fn timeout() -> Result<Option<Duration>, String> {
    match std::env::var(TIMEOUT) {
        Ok(value) => {
            let seconds = value
                .parse::<u64>()
                .map_err(|_| format!("{TIMEOUT} is invalid"))?;
            if seconds == 0 {
                Err(format!("{TIMEOUT} must be greater than zero"))
            } else {
                Ok(Some(Duration::from_secs(seconds)))
            }
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(unix)]
fn terminate(child: &mut Child) -> Result<(), String> {
    let pid = child.id();
    let pid = i32::try_from(pid).map_err(|_| "job process id is out of range".to_string())?;
    let result = unsafe { libc::kill(-pid, libc::SIGKILL) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(format!("failed to terminate job process group: {error}"))
    }
}

#[cfg(target_os = "windows")]
fn terminate(child: &mut Child) -> Result<(), String> {
    let system = std::env::var_os("SYSTEMROOT")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| "SYSTEMROOT is unavailable".to_string())?;
    let output = std::process::Command::new(system.join("System32").join("taskkill.exe"))
        .args(["/PID", &child.id().to_string(), "/T", "/F"])
        .output()
        .map_err(|error| format!("failed to invoke taskkill: {error}"))?;
    if output.status.success()
        || child
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_some()
    {
        Ok(())
    } else {
        Err(format!(
            "failed to terminate job process tree: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}
