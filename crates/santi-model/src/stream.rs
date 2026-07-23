use serde::{Deserialize, Serialize};

use crate::Timestamp;
use utoipa::ToSchema;

use santi_error::{Fault, Incident, Transition};

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
    StreamOpen,
    MessageCreated {
        message: crate::message::Placed,
    },
    MessageDelta {
        message: String,
        turn: String,
        role: crate::message::Role,
        text: String,
    },
    MessageCompleted {
        turn: String,
        message: crate::message::Placed,
    },
    ToolCallCreated {
        tool_call: crate::tool::Call,
    },
    ToolResultCreated {
        tool_result: crate::tool::Reply,
    },
    ThinkingCreated {
        thinking: crate::thinking::Span,
    },
    ThinkingUpdated {
        thinking: crate::thinking::Span,
    },
    ThinkingCompleted {
        thinking: crate::thinking::Span,
    },
    MaterialUpdated {
        material: crate::material::Updated,
    },
    TurnStarted {
        turn: crate::turn::Turn,
    },
    TurnActivity {
        activity: crate::turn::Activity,
    },
    TurnCompleted {
        turn: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    TurnFailed {
        turn: String,
        error: Box<Fault>,
    },
    Transition {
        transition: Box<Transition>,
    },
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
