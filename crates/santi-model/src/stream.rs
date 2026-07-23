use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::{
    ActorType, Compact, MaterialUpdated, MessageEvent, Strand, StrandEffect, StrandMessage,
    ThinkingSpan, Timestamp, ToolCall, ToolResult, Turn,
};
use santi_error::{Fault, Incident, Transition};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SantiStreamEvent {
    pub id: String,
    pub strand: String,
    pub created: Timestamp,
    pub payload: SantiStreamPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TurnActivityState {
    Requesting,
    Thinking,
    Generating,
    CallingTool,
    RunningTool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TurnActivity {
    pub turn: String,
    pub state: TurnActivityState,
    pub response: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SantiStreamPayload {
    StreamOpen,
    MessageCreated {
        message: StrandMessage,
    },
    MessageDelta {
        message: String,
        turn: String,
        role: ActorType,
        text: String,
    },
    MessageCompleted {
        turn: String,
        message: StrandMessage,
    },
    ToolCallCreated {
        tool_call: ToolCall,
    },
    ToolResultCreated {
        tool_result: ToolResult,
    },
    ThinkingCreated {
        thinking: ThinkingSpan,
    },
    ThinkingUpdated {
        thinking: ThinkingSpan,
    },
    ThinkingCompleted {
        thinking: ThinkingSpan,
    },
    MaterialUpdated {
        material: MaterialUpdated,
    },
    TurnStarted {
        turn: Turn,
    },
    TurnActivity {
        activity: TurnActivity,
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
pub struct StrandRuntimeSnapshot {
    pub strand: Strand,
    pub messages: Vec<StrandMessage>,
    pub events: Vec<MessageEvent>,
    pub turns: Vec<Turn>,
    pub thinking: Vec<ThinkingSpan>,
    pub calls: Vec<ToolCall>,
    pub results: Vec<ToolResult>,
    pub compacts: Vec<Compact>,
    pub effects: Vec<StrandEffect>,
    pub errors: Vec<Incident>,
}
