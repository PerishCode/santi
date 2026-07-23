use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::Timestamp;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = event::Batch)]
pub struct Batch {
    pub cursor: i64,
    pub events: Vec<Event>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = event::Event)]
pub struct Event {
    pub id: String,
    pub strand: String,
    pub turn: String,
    pub label: String,
    pub text: String,
    pub completed: Timestamp,
}
