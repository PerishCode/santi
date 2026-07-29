use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use santi_core::{
    job,
    service::{JobLaunch, JobObservation, JobSupervisor},
};
use std::os::windows::process::CommandExt;

use super::{
    DIRECTORY, STAMP, TIMEOUT, files, handoff,
    model::{Phase, Spec},
};

const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

pub struct Windows {
    executable: PathBuf,
}

impl Windows {
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
}

impl JobSupervisor for Windows {
    fn detach(&self, launch: &JobLaunch) -> Result<(), String> {
        let directory = Path::new(&launch.directory);
        files::directory(directory).prepare()?;
        let requested = Spec::from(launch);
        files::directory(directory).specify(&requested)?;
        let retained = files::directory(directory).spec()?;
        if files::directory(directory).terminal()?.is_some() {
            return Ok(());
        }
        if files::directory(directory).active()? {
            return if retained.legacy() {
                Ok(())
            } else {
                handoff(&launch.job.id, directory)
            };
        }
        if files::directory(directory).state()?.is_some() {
            return Err(format!(
                "job {} retains state without an active Windows sidecar",
                launch.job.id
            ));
        }

        Command::new(&self.executable)
            .args(["__job", "run"])
            .env(DIRECTORY, &launch.directory)
            .env(STAMP, &launch.stamp)
            .env(TIMEOUT, launch.job.timeout_seconds.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
            .spawn()
            .map_err(|error| format!("failed to launch Windows job sidecar: {error}"))?;
        handoff(&launch.job.id, directory)
    }

    fn observe(&self, launch: &JobLaunch) -> Result<JobObservation, String> {
        let directory = Path::new(&launch.directory);
        let state = files::directory(directory).state()?;
        if let Some(terminal) = files::directory(directory).terminal()? {
            if state.is_none()
                && launch.stamp.starts_with("stamp_")
                && launch.job.state == job::State::Submitting
            {
                return Ok(JobObservation::Aborted);
            }
            return Ok(JobObservation::Terminal(terminal));
        }
        if !files::directory(directory).active()? {
            return Ok(JobObservation::Missing);
        }
        if state.is_some_and(|state| state.phase == Phase::Claimed) {
            Ok(JobObservation::Claimed)
        } else {
            Ok(JobObservation::Running)
        }
    }

    fn stop(&self, launch: &JobLaunch) -> Result<(), String> {
        files::directory(Path::new(&launch.directory)).mark(files::CANCEL)
    }

    fn acknowledge(&self, launch: &JobLaunch) -> Result<(), String> {
        if files::directory(Path::new(&launch.directory)).active()? {
            Err(format!(
                "job {} Windows sidecar is still active",
                launch.job.id
            ))
        } else {
            Ok(())
        }
    }
}
