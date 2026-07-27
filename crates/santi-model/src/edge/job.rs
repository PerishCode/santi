use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::Timestamp;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[schema(as = job::State)]
pub enum State {
    Submitting,
    Accepted,
    Running,
    Cancelling,
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
    Unknown,
}

impl State {
    pub fn terminal(&self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::TimedOut | Self::Cancelled | Self::Unknown
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = job::Origin)]
pub struct Origin {
    pub soul: String,
    pub strand: String,
    pub turn: String,
    pub call: String,
    pub effect: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = job::Job)]
pub struct Job {
    pub id: String,
    pub origin: Origin,
    pub description: String,
    pub command: String,
    pub cwd: Option<String>,
    pub timeout_seconds: u64,
    pub output_limit_bytes: u64,
    #[serde(rename = "remind_every_seconds")]
    pub remind: Option<u64>,
    pub state: State,
    pub reason: Option<String>,
    pub exit_code: Option<i32>,
    pub created: Timestamp,
    pub updated: Timestamp,
    pub accepted: Option<Timestamp>,
    pub started: Option<Timestamp>,
    #[serde(rename = "last_reminded")]
    pub last: Option<Timestamp>,
    #[serde(rename = "next_reminder")]
    pub next: Option<Timestamp>,
    pub finished: Option<Timestamp>,
    pub acknowledged: Option<Timestamp>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = job::Accepted)]
pub struct Accepted {
    pub job: Job,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[schema(as = job::Stream)]
pub enum Stream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = job::Log)]
pub struct Log {
    pub job: String,
    pub stream: Stream,
    pub cursor: String,
    pub next: String,
    pub eof: bool,
    pub data: String,
}
