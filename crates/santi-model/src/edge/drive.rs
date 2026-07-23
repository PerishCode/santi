use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[schema(as = drive::State)]
pub enum State {
    Started,
    Running,
    Idle,
    Paused,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = drive::Response)]
pub struct Response {
    pub strand: String,
    pub state: State,
    pub turn: Option<crate::turn::Turn>,
}
