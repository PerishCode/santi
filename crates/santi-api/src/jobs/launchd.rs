use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
};

use santi_core::{
    job,
    service::{JobLaunch, JobObservation, JobSupervisor, JobTerminal},
};

use super::{
    DIRECTORY, GENERATION, STAMP, TIMEOUT, files, handoff,
    model::{Phase, Spec},
};

const PLIST: &str = "launchd.plist";

pub struct Launchd {
    executable: PathBuf,
    domain: String,
}

impl Launchd {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            domain: format!("gui/{}", unsafe { libc::geteuid() }),
        }
    }

    pub fn current() -> Result<Self, String> {
        std::env::current_exe()
            .map(Self::new)
            .map_err(|error| error.to_string())
    }

    fn target(&self, sidecar: &str) -> String {
        format!("{}/{sidecar}", self.domain)
    }

    fn matching(&self, launch: &JobLaunch) -> Result<bool, String> {
        let Some(held) = inspect(&self.target(&launch.sidecar))? else {
            return Ok(false);
        };
        Ok(stamp(&held).is_some_and(|stamp| stamp == launch.stamp))
    }

    fn plist(&self, launch: &JobLaunch) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{executable}</string>
    <string>__job</string>
    <string>run</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>{directory_key}</key>
    <string>{directory}</string>
    <key>{stamp_key}</key>
    <string>{stamp}</string>
    <key>{timeout_key}</key>
    <string>{timeout}</string>
  </dict>
  <key>RunAtLoad</key>
  <true/>
  <key>ProcessType</key>
  <string>Background</string>
  <key>ExitTimeOut</key>
  <integer>5</integer>
  <key>AbandonProcessGroup</key>
  <false/>
</dict>
</plist>
"#,
            label = xml(&launch.sidecar),
            executable = xml(&self.executable.display().to_string()),
            directory_key = DIRECTORY,
            directory = xml(&launch.directory),
            stamp_key = STAMP,
            stamp = xml(&launch.stamp),
            timeout_key = TIMEOUT,
            timeout = launch.job.timeout_seconds,
        )
    }
}

impl JobSupervisor for Launchd {
    fn detach(&self, launch: &JobLaunch) -> Result<(), String> {
        let directory = Path::new(&launch.directory);
        files::prepare(directory)?;
        let requested = Spec::from(launch);
        files::specify(directory, &requested)?;
        let retained = files::spec(directory)?;
        let plist = self.plist(launch);
        files::artifact(directory, PLIST, plist.as_bytes())?;
        if self.matching(launch)? {
            return if retained.legacy() {
                Ok(())
            } else {
                handoff(&launch.job.id, directory)
            };
        }

        let path = directory.join(PLIST);
        let output = Command::new("launchctl")
            .args(["bootstrap", &self.domain])
            .arg(&path)
            .output()
            .map_err(|error| format!("failed to invoke launchctl: {error}"))?;
        if output.status.success() || self.matching(launch)? {
            return handoff(&launch.job.id, directory);
        }
        Err(format!(
            "launchd did not accept job {}: {}",
            launch.job.id,
            diagnostic(&output)
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
        let Some(held) = inspect(&self.target(&launch.sidecar))? else {
            return Ok(JobObservation::Missing);
        };
        if !stamp(&held).is_some_and(|stamp| stamp == launch.stamp) {
            return Err(format!(
                "job {} sidecar stamp conflicts with retained service {}",
                launch.job.id, launch.sidecar
            ));
        }
        let active = held.get("state").map(String::as_str).unwrap_or("");
        if active == "running" {
            return if state
                .as_ref()
                .is_some_and(|state| state.phase == Phase::Claimed)
            {
                Ok(JobObservation::Claimed)
            } else {
                Ok(JobObservation::Running)
            };
        }
        let exit = held
            .get("last exit code")
            .and_then(|value| value.parse::<i32>().ok());
        match exit {
            Some(0) => Ok(JobObservation::Terminal(JobTerminal {
                state: job::State::Succeeded,
                reason: None,
                exit,
            })),
            Some(code) => Ok(JobObservation::Terminal(JobTerminal {
                state: job::State::Failed,
                reason: Some("supervisor_failed".to_string()),
                exit: Some(code),
            })),
            None => Ok(JobObservation::Claimed),
        }
    }

    fn stop(&self, launch: &JobLaunch) -> Result<(), String> {
        files::mark(Path::new(&launch.directory), files::CANCEL)
    }

    fn acknowledge(&self, launch: &JobLaunch) -> Result<(), String> {
        bootout(&self.target(&launch.sidecar))
    }
}

fn inspect(target: &str) -> Result<Option<BTreeMap<String, String>>, String> {
    let output = Command::new("launchctl")
        .args(["print", target])
        .output()
        .map_err(|error| format!("failed to invoke launchctl: {error}"))?;
    if !output.status.success() {
        let message = diagnostic(&output);
        return if missing(&message) {
            Ok(None)
        } else {
            Err(format!("launchctl could not inspect {target}: {message}"))
        };
    }
    let mut held = BTreeMap::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let line = line.trim();
        if let Some((key, value)) = line.split_once(" = ") {
            held.insert(key.to_string(), value.to_string());
        } else if let Some((key, value)) = line.split_once(" => ") {
            held.insert(format!("environment.{key}"), value.to_string());
        }
    }
    Ok(Some(held))
}

fn stamp(held: &BTreeMap<String, String>) -> Option<&str> {
    held.get(&format!("environment.{STAMP}"))
        .or_else(|| held.get(&format!("environment.{GENERATION}")))
        .map(String::as_str)
}

fn bootout(target: &str) -> Result<(), String> {
    let output = Command::new("launchctl")
        .args(["bootout", target])
        .output()
        .map_err(|error| format!("failed to invoke launchctl: {error}"))?;
    let message = diagnostic(&output);
    if output.status.success() || missing(&message) {
        Ok(())
    } else {
        Err(format!("launchctl bootout {target} failed: {message}"))
    }
}

fn missing(message: &str) -> bool {
    message.contains("Could not find service")
        || message.contains("No such process")
        || message.contains("not found")
}

fn diagnostic(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let message = stderr.trim();
    if message.is_empty() {
        stdout.trim().to_string()
    } else {
        message.to_string()
    }
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
