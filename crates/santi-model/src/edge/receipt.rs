use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::Timestamp;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[schema(as = receipt::State)]
pub enum State {
    Accepted,
    Recovered,
    Driving,
    Failed,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = receipt::Transition)]
pub struct Transition {
    pub id: String,
    pub sequence: i64,
    pub state: State,
    pub turn: Option<String>,
    pub incident: Option<String>,
    pub rebuilt: Option<String>,
    pub occurred: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = receipt::Status)]
pub struct Status {
    pub inbox: String,
    pub strand: String,
    pub state: State,
    pub accepted: Timestamp,
    pub updated: Timestamp,
    pub transitions: Vec<Transition>,
    pub effects: Vec<crate::effect::Effect>,
}
