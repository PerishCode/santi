use serde::{Deserialize, Serialize};

use crate::Timestamp;
use utoipa::ToSchema;

use santi_error::{Incident, Transition};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = stream::Event)]
pub struct Event {
    pub id: String,
    pub strand: String,
    pub created: Timestamp,
    pub payload: Payload,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
#[schema(as = stream::Payload)]
pub enum Payload {
    Open,
    Message(crate::message::Beat),
    Tool(crate::tool::Beat),
    Thinking(crate::thinking::Beat),
    Turn(crate::turn::Beat),
    Material(crate::material::Beat),
    Transition { transition: Box<Transition> },
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = stream::Snapshot)]
pub struct Snapshot {
    pub strand: crate::strand::Strand,
    pub messages: Vec<crate::message::Placed>,
    pub events: Vec<crate::message::Event>,
    pub turns: Vec<crate::turn::Turn>,
    pub thinking: Vec<crate::thinking::Span>,
    pub calls: Vec<crate::tool::Call>,
    pub results: Vec<crate::tool::Reply>,
    pub compacts: Vec<crate::compact::Compact>,
    pub effects: Vec<crate::effect::Effect>,
    pub errors: Vec<Incident>,
}
