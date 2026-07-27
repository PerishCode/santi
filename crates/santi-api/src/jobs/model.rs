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
    pub generation: String,
    pub unit: String,
    pub description: String,
    pub command: String,
    pub cwd: String,
    #[serde(rename = "output_limit_bytes")]
    pub output: u64,
}

impl Spec {
    pub(super) fn from(launch: &JobLaunch) -> Self {
        Self {
            schema: "santi.job.execution.v1".to_string(),
            id: launch.job.id.clone(),
            origin: launch.job.origin.clone(),
            generation: launch.generation.clone(),
            unit: launch.supervisor.clone(),
            description: launch.job.description.clone(),
            command: launch.job.command.clone(),
            cwd: launch.cwd.clone(),
            output: launch.job.output_limit_bytes,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Record {
    pub state: job::State,
    pub reason: Option<String>,
    #[serde(rename = "exit_code")]
    pub exit: Option<i32>,
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
