mod files;
#[cfg(target_os = "macos")]
mod launchd;
mod model;
mod process;
#[cfg(target_os = "linux")]
mod systemd;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "macos")]
pub use launchd::Launchd;
#[cfg(target_os = "macos")]
pub use launchd::Launchd as Native;
#[cfg(target_os = "linux")]
pub use systemd::Systemd;
#[cfg(target_os = "linux")]
pub use systemd::Systemd as Native;
#[cfg(target_os = "windows")]
pub use windows::Windows;
#[cfg(target_os = "windows")]
pub use windows::Windows as Native;

use std::{
    path::Path,
    thread,
    time::{Duration, Instant},
};

pub(super) const DIRECTORY: &str = "SANTI_JOB_DIRECTORY";
pub(super) const GENERATION: &str = "SANTI_JOB_GENERATION";
pub(super) const STAMP: &str = "SANTI_JOB_STAMP";
pub(super) const TIMEOUT: &str = "SANTI_JOB_TIMEOUT_SECONDS";

const HANDOFF: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(20);

fn handoff(id: &str, directory: &Path) -> Result<(), String> {
    let start = Instant::now();
    loop {
        if files::state(directory)?.is_some() {
            return Ok(());
        }
        if files::terminal(directory)?.is_some() {
            return Err(format!("job {id} sidecar failed before claimed handoff"));
        }
        if start.elapsed() >= HANDOFF {
            return Err(format!(
                "job {id} sidecar did not claim the detached handoff"
            ));
        }
        thread::sleep(POLL);
    }
}

pub fn run() -> Result<(), String> {
    process::run()
}

pub fn finalize() -> Result<(), String> {
    process::finalize()
}
