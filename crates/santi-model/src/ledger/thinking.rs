use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::Timestamp;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[schema(as = thinking::State)]
pub enum State {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[schema(as = thinking::Reason)]
pub enum Reason {
    Spoke,
    Called,
    Finished,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = thinking::Span)]
pub struct Span {
    pub id: String,
    pub turn: String,
    pub response: Option<String>,
    pub state: State,
    pub summary: Option<String>,
    pub completion_reason: Option<Reason>,
    pub error: Option<String>,
    pub created: Timestamp,
    pub updated: Timestamp,
    pub finished: Option<Timestamp>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "beat", rename_all = "snake_case")]
#[schema(as = thinking::Beat)]
pub enum Beat {
    Created { thinking: Span },
    Updated { thinking: Span },
    Completed { thinking: Span },
}
