use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::Timestamp;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[schema(as = turn::Trigger)]
pub enum Trigger {
    StrandSend,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[schema(as = turn::Status)]
pub enum Status {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = turn::Turn)]
pub struct Turn {
    pub id: String,
    pub strand: String,
    pub trigger: Trigger,
    pub source: Option<String>,
    pub from: i64,
    pub to: Option<i64>,
    pub status: Status,
    pub error: Option<String>,
    pub created: Timestamp,
    pub updated: Timestamp,
    pub finished: Option<Timestamp>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = turn::Motion)]
pub enum Motion {
    Requesting,
    Thinking,
    Generating,
    Calling,
    Running,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = turn::Activity)]
pub struct Activity {
    pub turn: String,
    pub state: Motion,
    pub response: Option<String>,
}
