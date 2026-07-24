use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::Timestamp;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[schema(as = effect::Outcome)]
pub enum Outcome {
    Applied,
    NotApplied,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[schema(as = effect::State)]
pub enum State {
    Prepared,
    Dispatching,
    Unknown,
    Settled(Outcome),
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = effect::Effect)]
pub struct Effect {
    pub id: String,
    pub strand: String,
    pub turn: String,
    pub call: Option<String>,
    pub kind: String,
    pub state: State,
    pub result: Option<String>,
    pub error: Option<String>,
    pub created: Timestamp,
    pub updated: Timestamp,
    pub dispatched: Option<Timestamp>,
    pub settled: Option<Timestamp>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = effect::Status)]
pub struct Status {
    pub effect: Effect,
    pub receipts: Vec<String>,
}
