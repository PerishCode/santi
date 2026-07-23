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

mod stream;
pub use stream::*;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    pub ok: bool,
    pub service: String,
    pub degraded: bool,
    pub incidents: i64,
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
    pub strand: String,
    pub kind: MaterialKind,
    pub content_type: String,
    pub text: String,
    pub updated: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MaterialUpdated {
    pub strand: String,
    pub kind: MaterialKind,
    pub updated: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Soul {
    pub id: String,
    pub created: Timestamp,
    pub updated: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Strand {
    pub id: String,
    pub soul: String,
    pub label: Option<String>,
    pub memory: String,
    pub state: Option<Value>,
    pub next: i64,
    pub seen: i64,
    pub parent: Option<String>,
    pub fork: Option<i64>,
    pub created: Timestamp,
    pub updated: Timestamp,
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
    pub strand: String,
    pub trigger: TurnTriggerType,
    pub source: Option<String>,
    pub from: i64,
    pub to: Option<i64>,
    pub status: TurnStatus,
    pub error: Option<String>,
    pub created: Timestamp,
    pub updated: Timestamp,
    pub finished: Option<Timestamp>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ToolCall {
    pub id: String,
    pub turn: String,
    pub tool: String,
    pub arguments: Value,
    pub created: Timestamp,
}

#[derive(Debug, Clone, Default)]
pub struct ToolCallProvenance {
    pub family: String,
    pub item: Option<Value>,
    pub mark: Option<String>,
    pub response_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ToolResult {
    pub id: String,
    pub call: String,
    pub output: Option<Value>,
    pub error: Option<String>,
    pub created: Timestamp,
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
    pub turn: String,
    pub response: Option<String>,
    pub state: ThinkingSpanState,
    pub summary: Option<String>,
    pub completion_reason: Option<ThinkingCompletionReason>,
    pub error: Option<String>,
    pub created: Timestamp,
    pub updated: Timestamp,
    pub finished: Option<Timestamp>,
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
    pub strand: String,
    pub kind: StrandTargetType,
    pub target: String,
    pub seq: i64,
    pub created: Timestamp,
}

pub fn now() -> Timestamp {
    use jiff::fmt::temporal::DateTimePrinter;

    let now = jiff::Timestamp::now();
    let mut buf = String::new();
    DateTimePrinter::new()
        .precision(Some(3))
        .print_timestamp(&now, &mut buf)
        .expect("formatting a timestamp into a String cannot fail");
    buf
}

pub fn stamped(system_time: std::time::SystemTime) -> Result<Timestamp, String> {
    use jiff::fmt::temporal::DateTimePrinter;

    let timestamp = jiff::Timestamp::try_from(system_time).map_err(|error| error.to_string())?;
    let mut buf = String::new();
    DateTimePrinter::new()
        .precision(Some(3))
        .print_timestamp(&timestamp, &mut buf)
        .expect("formatting a timestamp into a String cannot fail");
    Ok(buf)
}

pub fn tag(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::new_v4().simple())
}
