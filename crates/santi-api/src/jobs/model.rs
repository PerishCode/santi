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
        self.id == other.id
            && self.origin.soul == other.origin.soul
            && self.origin.strand == other.origin.strand
            && self.origin.turn == other.origin.turn
            && self.origin.call == other.origin.call
            && self.origin.effect == other.origin.effect
            && self.stamp == other.stamp
            && self.sidecar == other.sidecar
            && self.description == other.description
            && self.command == other.command
            && self.cwd == other.cwd
            && self.output == other.output
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Record {
    pub state: job::State,
    pub reason: Option<String>,
    #[serde(rename = "exit_code")]
    pub exit: Option<i32>,
}

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

impl From<Record> for JobTerminal {
    fn from(record: Record) -> Self {
        Self {
            state: record.state,
            reason: record.reason,
            exit: record.exit,
        }
    }
}
