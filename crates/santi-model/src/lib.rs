use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

mod api;
mod codec;
pub use api::*;

pub type Timestamp = String;

mod message;
pub use message::*;

mod budget;
pub use budget::*;

mod compact;
pub use compact::*;

mod effects;
pub use effects::*;

mod im;
pub use im::*;

mod stream;
pub use stream::*;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    pub ok: bool,
    pub service: String,
    pub degraded: bool,
    pub active_drive_incidents: i64,
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
pub struct StrandMaterial {
    pub strand_id: String,
    pub kind: MaterialKind,
    pub content_type: String,
    pub text: String,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MaterialUpdated {
    pub strand_id: String,
    pub kind: MaterialKind,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Soul {
    pub id: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Strand {
    pub id: String,
    pub soul_id: String,
    pub external_label: Option<String>,
    pub strand_memory: String,
    pub provider_state: Option<Value>,
    pub next_seq: i64,
    pub last_seen_strand_seq: i64,
    pub parent_strand_id: Option<String>,
    pub fork_point: Option<i64>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnTriggerType {
    StrandSend,
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
    pub strand_id: String,
    pub trigger_type: TurnTriggerType,
    pub trigger_ref: Option<String>,
    pub base_strand_seq: i64,
    pub end_strand_seq: Option<i64>,
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

#[derive(Debug, Clone, Default)]
pub struct ToolCallProvenance {
    pub provider_family: String,
    pub item: Option<Value>,
    pub item_id: Option<String>,
    pub response_id: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StrandTargetType {
    Message,
    Compact,
    Thinking,
    ToolCall,
    ToolResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StrandEntry {
    pub strand_id: String,
    pub target_type: StrandTargetType,
    pub target_id: String,
    pub strand_seq: i64,
    pub created_at: Timestamp,
}

pub fn timestamp_now() -> Timestamp {
    use jiff::fmt::temporal::DateTimePrinter;

    let now = jiff::Timestamp::now();
    let mut buf = String::new();
    DateTimePrinter::new()
        .precision(Some(3))
        .print_timestamp(&now, &mut buf)
        .expect("formatting a timestamp into a String cannot fail");
    buf
}

pub fn timestamp_from_system_time(system_time: std::time::SystemTime) -> Result<Timestamp, String> {
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
