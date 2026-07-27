use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

use santi_core::{
    job,
    service::{JobLaunch, JobObservation, JobSupervisor, JobTerminal},
};

use super::{
    files,
    model::{Phase, Spec},
};

pub(super) const DIRECTORY: &str = "SANTI_JOB_DIRECTORY";
pub(super) const GENERATION: &str = "SANTI_JOB_GENERATION";
pub(super) const STAMP: &str = "SANTI_JOB_STAMP";

const HANDOFF: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(20);

pub struct Systemd {
    executable: PathBuf,
}

impl Systemd {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    pub fn current() -> Result<Self, String> {
        std::env::current_exe()
            .map(Self::new)
            .map_err(|error| error.to_string())
    }

    fn matching(&self, launch: &JobLaunch) -> Result<bool, String> {
        let held = properties(&launch.sidecar)?;
        if held.get("LoadState").map(String::as_str) == Some("not-found") {
            return Ok(false);
        }
        Ok(held.get("Environment").is_some_and(|environment| {
            environment.contains(&format!("{STAMP}={}", launch.stamp))
                || environment.contains(&format!("{GENERATION}={}", launch.stamp))
        }))
    }

    fn handoff(&self, launch: &JobLaunch) -> Result<(), String> {
        let directory = Path::new(&launch.directory);
        let start = Instant::now();
        loop {
            if files::state(directory)?.is_some() {
                return Ok(());
            }
            if files::terminal(directory)?.is_some() {
                return Err(format!(
                    "job {} sidecar failed before claimed handoff",
                    launch.job.id
                ));
            }
            if start.elapsed() >= HANDOFF {
                return Err(format!(
                    "job {} sidecar did not claim the detached handoff",
                    launch.job.id
                ));
            }
            thread::sleep(POLL);
        }
    }
}

impl JobSupervisor for Systemd {
    fn detach(&self, launch: &JobLaunch) -> Result<(), String> {
        let directory = Path::new(&launch.directory);
        files::prepare(directory)?;
        let requested = Spec::from(launch);
        files::specify(directory, &requested)?;
        let retained = files::spec(directory)?;
        if self.matching(launch)? {
            return if retained.legacy() {
                Ok(())
            } else {
                self.handoff(launch)
            };
        }

        let stop = format!(
            "{} __job finalize",
            quote(&self.executable.display().to_string())
        );
        let output = Command::new("systemd-run")
            .args(["--user", "--no-block"])
            .arg(format!("--unit={}", launch.sidecar))
            .arg("--property=Type=oneshot")
            .arg("--property=RemainAfterExit=yes")
            .arg("--property=KillMode=control-group")
            .arg("--property=TimeoutStopSec=5s")
            .arg(format!(
                "--property=TimeoutStartSec={}s",
                launch.job.timeout_seconds
            ))
            .arg(format!("--property=ExecStopPost={stop}"))
            .arg(format!("--setenv={DIRECTORY}={}", launch.directory))
            .arg(format!("--setenv={STAMP}={}", launch.stamp))
            .arg(&self.executable)
            .args(["__job", "run"])
            .output()
            .map_err(|error| format!("failed to invoke systemd-run: {error}"))?;
        if output.status.success() || self.matching(launch)? {
            return self.handoff(launch);
        }
        Err(format!(
            "systemd did not accept job {}: {}",
            launch.job.id,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }

    fn observe(&self, launch: &JobLaunch) -> Result<JobObservation, String> {
        let directory = Path::new(&launch.directory);
        let state = files::state(directory)?;
        if let Some(terminal) = files::terminal(directory)? {
            if state.is_none()
                && launch.stamp.starts_with("stamp_")
                && launch.job.state == job::State::Submitting
            {
                return Ok(JobObservation::Aborted);
            }
            return Ok(JobObservation::Terminal(terminal.into()));
        }
        let held = properties(&launch.sidecar)?;
        if held.get("LoadState").map(String::as_str) == Some("not-found") {
            return Ok(JobObservation::Missing);
        }
        if !held.get("Environment").is_some_and(|environment| {
            environment.contains(&format!("{STAMP}={}", launch.stamp))
                || environment.contains(&format!("{GENERATION}={}", launch.stamp))
        }) {
            return Err(format!(
                "job {} sidecar stamp conflicts with retained unit {}",
                launch.job.id, launch.sidecar
            ));
        }
        let active = held.get("ActiveState").map(String::as_str).unwrap_or("");
        let sub = held.get("SubState").map(String::as_str).unwrap_or("");
        let result = held.get("Result").map(String::as_str).unwrap_or("");
        let status = held
            .get("ExecMainStatus")
            .and_then(|value| value.parse::<i32>().ok());
        let limited = directory.join(files::LIMIT).is_file();
        let cancelled = directory.join(files::CANCEL).is_file();
        match (active, sub, result) {
            ("activating", _, _) | ("active", "running", _)
                if state
                    .as_ref()
                    .is_some_and(|state| state.phase == Phase::Claimed) =>
            {
                Ok(JobObservation::Claimed)
            }
            ("activating", _, _) | ("active", "running", _) => Ok(JobObservation::Running),
            ("active", "exited", "success") => Ok(JobObservation::Terminal(JobTerminal {
                state: job::State::Succeeded,
                reason: None,
                exit: Some(0),
            })),
            ("failed", _, _) if limited => Ok(JobObservation::Terminal(JobTerminal {
                state: job::State::Failed,
                reason: Some("output_limit".to_string()),
                exit: status,
            })),
            ("failed", _, _) if cancelled => Ok(JobObservation::Terminal(JobTerminal {
                state: job::State::Cancelled,
                reason: Some("cancel_requested".to_string()),
                exit: status,
            })),
            ("failed", _, "timeout") => Ok(JobObservation::Terminal(JobTerminal {
                state: job::State::TimedOut,
                reason: Some("runtime_limit".to_string()),
                exit: status,
            })),
            ("failed", _, _) => Ok(JobObservation::Terminal(JobTerminal {
                state: job::State::Failed,
                reason: Some(if result.is_empty() {
                    "supervisor_failed".to_string()
                } else {
                    result.to_string()
                }),
                exit: status,
            })),
            ("deactivating", _, _) => Ok(JobObservation::Running),
            _ => Ok(JobObservation::Claimed),
        }
    }

    fn stop(&self, launch: &JobLaunch) -> Result<(), String> {
        files::mark(Path::new(&launch.directory), files::CANCEL)?;
        control(&["stop", &launch.sidecar], true)
    }

    fn acknowledge(&self, launch: &JobLaunch) -> Result<(), String> {
        control(&["stop", &launch.sidecar], false)?;
        control(&["reset-failed", &launch.sidecar], false)
    }
}

fn properties(unit: &str) -> Result<BTreeMap<String, String>, String> {
    let output = Command::new("systemctl")
        .args([
            "--user",
            "show",
            unit,
            "--property=LoadState",
            "--property=ActiveState",
            "--property=SubState",
            "--property=Result",
            "--property=ExecMainStatus",
            "--property=Environment",
            "--no-pager",
        ])
        .output()
        .map_err(|error| format!("failed to invoke systemctl: {error}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut held = BTreeMap::new();
    for line in text.lines() {
        if let Some((key, value)) = line.split_once('=') {
            held.insert(key.to_string(), value.to_string());
        }
    }
    if output.status.success() || held.get("LoadState").map(String::as_str) == Some("not-found") {
        Ok(held)
    } else {
        Err(format!(
            "systemctl could not inspect {unit}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

pub(super) fn control(arguments: &[&str], strict: bool) -> Result<(), String> {
    let output = Command::new("systemctl")
        .arg("--user")
        .args(arguments)
        .output()
        .map_err(|error| format!("failed to invoke systemctl: {error}"))?;
    if output.status.success()
        || !strict
            && (String::from_utf8_lossy(&output.stderr).contains("not loaded")
                || String::from_utf8_lossy(&output.stderr).contains("not found"))
    {
        Ok(())
    } else {
        Err(format!(
            "systemctl {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}
