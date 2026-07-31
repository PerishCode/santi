use santi_core::{
    job,
    service::{JobLaunch, JobTerminal},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Spec {
    pub schema: String,
    pub id: String,
    pub origin: job::Origin,
    #[serde(alias = "generation")]
    pub stamp: String,
    #[serde(alias = "unit")]
    pub sidecar: String,
    pub description: String,
    pub command: String,
    pub cwd: String,
    #[serde(rename = "output_limit_bytes")]
    pub output: u64,
}

#[derive(PartialEq, Eq)]
struct Identity<'a> {
    id: &'a str,
    origin: &'a job::Origin,
    stamp: &'a str,
    sidecar: &'a str,
    description: &'a str,
    command: &'a str,
    cwd: &'a str,
    output: u64,
}

impl Spec {
    pub(super) fn from(launch: &JobLaunch) -> Self {
        Self {
            schema: "santi.job.execution.v2".to_string(),
            id: launch.job.id.clone(),
            origin: launch.job.origin.clone(),
            stamp: launch.stamp.clone(),
            sidecar: launch.sidecar.clone(),
            description: launch.job.description.clone(),
            command: launch.job.command.clone(),
            cwd: launch.cwd.clone(),
            output: launch.job.output_limit_bytes,
        }
    }

    pub(super) fn legacy(&self) -> bool {
        self.schema == "santi.job.execution.v1"
    }

    pub(super) fn matches(&self, other: &Self) -> bool {
        self.identity() == other.identity()
    }

    fn identity(&self) -> Identity<'_> {
        Identity {
            id: &self.id,
            origin: &self.origin,
            stamp: &self.stamp,
            sidecar: &self.sidecar,
            description: &self.description,
            command: &self.command,
            cwd: &self.cwd,
            output: self.output,
        }
    }
}

pub(super) type Record = JobTerminal;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Phase {
    Claimed,
    Running,
    Terminal,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Snapshot {
    pub schema: String,
    pub phase: Phase,
}

impl Snapshot {
    pub(super) fn new(phase: Phase) -> Self {
        Self {
            schema: "santi.job.state.v1".to_string(),
            phase,
        }
    }
}
