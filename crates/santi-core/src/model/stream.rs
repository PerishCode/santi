use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::{
    ActorType, Compact, MaterialUpdated, MessageEvent, Strand, StrandEffect, StrandMessage,
    ThinkingSpan, Timestamp, ToolCall, ToolResult, Turn,
};
use crate::{ErrorIncident, ErrorTransition};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SantiStreamEvent {
    pub event_id: String,
    pub strand_id: String,
    pub created_at: Timestamp,
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
    pub turn_id: String,
    pub state: TurnActivityState,
    pub provider_response_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SantiStreamPayload {
    StreamOpen,
    MessageCreated {
        message: StrandMessage,
    },
    MessageDelta {
        message_id: String,
        turn_id: String,
        role: ActorType,
        text: String,
    },
    MessageCompleted {
        turn_id: String,
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
        turn_id: String,
    },
    TurnFailed {
        turn_id: String,
        error: String,
    },
    ErrorTransition {
        transition: Box<ErrorTransition>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StrandRuntimeSnapshot {
    pub strand: Strand,
    pub messages: Vec<StrandMessage>,
    pub message_events: Vec<MessageEvent>,
    pub turns: Vec<Turn>,
    pub thinking_spans: Vec<ThinkingSpan>,
    pub tool_calls: Vec<ToolCall>,
    pub tool_results: Vec<ToolResult>,
    pub compacts: Vec<Compact>,
    pub effects: Vec<StrandEffect>,
    pub errors: Vec<ErrorIncident>,
}
