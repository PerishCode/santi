use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

pub type Timestamp = String;

mod message;
pub use message::*;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    pub ok: bool,
    pub service: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ErrorResponse {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MaterialKind {
    SystemPrompt,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MaterialRequest {
    pub kind: MaterialKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SessionMaterial {
    pub session_id: String,
    pub kind: MaterialKind,
    pub content_type: String,
    pub text: String,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MaterialUpdated {
    pub session_id: String,
    pub kind: MaterialKind,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Session {
    pub id: String,
    pub parent_session_id: Option<String>,
    pub fork_point: Option<i64>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SessionProfile {
    pub session_id: String,
    pub title: Option<String>,
    pub desc: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SessionSummary {
    pub session: Session,
    pub profile: SessionProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Soul {
    pub id: String,
    pub memory: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SoulProfile {
    pub soul_id: String,
    pub soul_name: String,
    pub nickname: String,
    pub avatar_ref: Option<String>,
    pub avatar_seed: String,
    pub desc: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SoulSession {
    pub id: String,
    pub soul_id: String,
    pub session_id: String,
    pub session_memory: String,
    pub provider_state: Option<Value>,
    pub next_seq: i64,
    pub last_seen_session_seq: i64,
    pub parent_soul_session_id: Option<String>,
    pub fork_point: Option<i64>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnTriggerType {
    SessionSend,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Turn {
    pub id: String,
    pub soul_session_id: String,
    pub trigger_type: TurnTriggerType,
    pub trigger_ref: Option<String>,
    pub input_through_session_seq: i64,
    pub base_soul_session_seq: i64,
    pub end_soul_session_seq: Option<i64>,
    pub status: TurnStatus,
    pub error_text: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub finished_at: Option<Timestamp>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ToolCall {
    pub id: String,
    pub turn_id: String,
    pub tool_name: String,
    pub arguments: Value,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ToolResult {
    pub id: String,
    pub tool_call_id: String,
    pub output: Option<Value>,
    pub error_text: Option<String>,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingSpanState {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingCompletionReason {
    FirstTextDelta,
    ToolCallRequested,
    ProviderCompleted,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ThinkingSpan {
    pub id: String,
    pub turn_id: String,
    pub provider_response_id: Option<String>,
    pub state: ThinkingSpanState,
    pub summary: Option<String>,
    pub completion_reason: Option<ThinkingCompletionReason>,
    pub error_text: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub finished_at: Option<Timestamp>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Compact {
    pub id: String,
    pub turn_id: String,
    pub summary: String,
    pub start_session_seq: i64,
    pub end_session_seq: i64,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SessionEffect {
    pub id: String,
    pub session_id: String,
    pub effect_type: String,
    pub idempotency_key: String,
    pub status: String,
    pub source_hook_id: String,
    pub source_turn_id: String,
    pub result_ref: Option<String>,
    pub error_text: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SoulSessionTargetType {
    Message,
    Compact,
    Thinking,
    ToolCall,
    ToolResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SoulSessionEntry {
    pub soul_session_id: String,
    pub target_type: SoulSessionTargetType,
    pub target_id: String,
    pub soul_session_seq: i64,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateSessionResponse {
    pub session: SessionSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SessionDetail {
    pub session: Session,
    pub profile: SessionProfile,
    pub messages: Vec<SessionMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateSessionRequest {
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SendSessionRequest {
    pub content: Vec<MessagePart>,
}

impl SendSessionRequest {
    pub fn text(&self) -> String {
        MessageContent {
            parts: self.content.clone(),
        }
        .content_text()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SendSessionAcceptedResponse {
    pub session: SessionSummary,
    pub soul_session: SoulSession,
    pub soul_profile: SoulProfile,
    pub turn: Turn,
    pub user_message: SessionMessage,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SantiStreamEvent {
    pub event_id: String,
    pub session_id: String,
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
        message: SessionMessage,
    },
    MessageDelta {
        message_id: String,
        turn_id: String,
        role: ActorType,
        text: String,
    },
    MessageCompleted {
        turn_id: String,
        message: SessionMessage,
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
    TurnFailed {
        turn_id: String,
        error: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SessionRuntimeSnapshot {
    pub session: Session,
    pub profile: SessionProfile,
    pub soul_session: Option<SoulSession>,
    pub soul_profile: Option<SoulProfile>,
    pub messages: Vec<SessionMessage>,
    pub turns: Vec<Turn>,
    pub thinking_spans: Vec<ThinkingSpan>,
    pub tool_calls: Vec<ToolCall>,
    pub tool_results: Vec<ToolResult>,
    pub compacts: Vec<Compact>,
    pub effects: Vec<SessionEffect>,
}

pub fn timestamp_now() -> Timestamp {
    use jiff::fmt::temporal::DateTimePrinter;

    // RFC3339 / ISO 8601 UTC with fixed millisecond precision. Fixed-width
    // fractional digits keep the string lexicographically sortable, which the
    // store and the browser projection both rely on (timestamps are used as
    // `ORDER BY` / `localeCompare` sort keys). A `jiff::Timestamp` is UTC, so
    // the printed form ends in `Z`.
    let now = jiff::Timestamp::now();
    let mut buf = String::new();
    DateTimePrinter::new()
        .precision(Some(3))
        .print_timestamp(&now, &mut buf)
        .expect("formatting a timestamp into a String cannot fail");
    buf
}

pub(crate) fn timestamp_from_system_time(
    system_time: std::time::SystemTime,
) -> Result<Timestamp, String> {
    use jiff::fmt::temporal::DateTimePrinter;

    let timestamp = jiff::Timestamp::try_from(system_time).map_err(|error| error.to_string())?;
    let mut buf = String::new();
    DateTimePrinter::new()
        .precision(Some(3))
        .print_timestamp(&timestamp, &mut buf)
        .expect("formatting a timestamp into a String cannot fail");
    Ok(buf)
}

pub fn prefixed_id(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::new_v4().simple())
}
