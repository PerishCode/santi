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

/// A soul is a cyber-individual, keyed by id alone. It has no name/avatar/desc
/// column: identity is the mutable self, and it lives entirely in the soul's
/// memory (rendered live into `[santi-soul]`), not in a profile row. The
/// timestamps are pure provenance.
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
    /// Opaque external anchor (e.g. a webhook thread key). Unique per soul;
    /// absent for strands reached only by id (e.g. CLI-created ones).
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
    /// The provider's raw function_call item (replayed verbatim by the Responses
    /// adapter). Null for older rows / providers that don't surface one.
    pub provider_item: Option<Value>,
    pub item_id: Option<String>,
    pub response_id: Option<String>,
    pub created_at: Timestamp,
}

/// Provider provenance captured for a tool call so the call can be replayed
/// faithfully (the Responses adapter echoes the raw `item`). All optional —
/// chat_completions and older rows may have none.
#[derive(Debug, Clone, Default)]
pub struct ToolCallProvenance {
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

/// A compact is a pure projection overlay over a strand's spine. It
/// self-describes its coverage by message-id boundaries (fork-safe) and carries
/// the soul-authored summary. The spine is never annotated. Provenance lives in
/// the audit log (the compact-exec tool_call), not here.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Compact {
    pub id: String,
    pub strand_id: String,
    pub summary: String,
    pub start_message_id: String,
    pub end_message_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CompactExecRequest {
    /// Range boundaries — must be FIXED user/assistant messages in this
    /// strand's spine. Everything between (messages/tools/reasoning) collapses.
    pub from_message_id: String,
    pub to_message_id: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CompactExecResponse {
    pub compact_id: String,
    pub start_message_id: String,
    pub end_message_id: String,
    /// Compacts fully covered by this range, dropped and replaced by the new one.
    pub absorbed: Vec<String>,
    /// Spine entries the new compact collapses out of the assembled view.
    pub collapsed_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CompactQueryEntry {
    pub strand_seq: i64,
    pub target_type: StrandTargetType,
    pub target_id: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CompactQueryResponse {
    pub compact_id: String,
    pub start_message_id: String,
    pub end_message_id: String,
    pub total: i64,
    pub page_index: i64,
    pub page_size: i64,
    pub entries: Vec<CompactQueryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StrandEffect {
    pub id: String,
    pub strand_id: String,
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

/// Create a new soul (an individual). Souls are API-managed, never config.
/// A soul is id-only; its identity is its memory, so the only thing to supply
/// at creation is the initial `[santi-soul]` memory to seed (empty/absent → a
/// blank soul that will author its own).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateSoulRequest {
    #[serde(default)]
    pub memory: Option<String>,
}

/// An API-managed webhook subscription: how an external source reaches a soul.
/// `adaptor` selects the boundary normalizer (integration knowledge); `soul_id`
/// is who receives the resulting turn; `strand_strategy` picks where the thread
/// lives (`per_thread` = one strand per adaptor-derived label, `single` = one
/// strand per subscription); `secret_env` names the env var holding the signing
/// secret (the secret itself is never stored). The `name` is the URL path segment.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WebhookSubscription {
    pub name: String,
    pub adaptor: String,
    pub soul_id: String,
    pub strand_strategy: String,
    pub secret_env: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateWebhookRequest {
    pub name: String,
    pub adaptor: String,
    pub soul_id: String,
    /// `per_thread` (default) or `single`.
    #[serde(default)]
    pub strand_strategy: Option<String>,
    pub secret_env: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateStrandResponse {
    pub strand: Strand,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StrandDetail {
    pub strand: Strand,
    pub messages: Vec<StrandMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SendStrandRequest {
    pub content: Vec<MessagePart>,
}

impl SendStrandRequest {
    pub fn text(&self) -> String {
        MessageContent {
            parts: self.content.clone(),
        }
        .content_text()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SendStrandAcceptedResponse {
    pub strand: Strand,
    pub turn: Turn,
    /// The content this send just enqueued, once the driver has actually
    /// committed it to the timeline. Absent when this send coalesced into an
    /// already-running turn — durably enqueued, but the driver has not drained
    /// it yet (it will, when that turn completes and re-pokes).
    pub user_message: Option<StrandMessage>,
}

/// How an ingest adaptor addresses a strand. Resolution is atomic (see
/// `SantiStore::resolve_strand_selector`) — the STRATEGY is the adaptor's: the
/// operator addresses an already-existing strand by id; a webhook addresses
/// one by an opaque label, scoped to its soul (find-or-create).
#[derive(Debug, Clone)]
pub enum StrandSelector {
    ById(String),
    ByLabel { soul_id: String, label: String },
}

/// The result of `ingest` — the one inbound path (a send, a webhook event).
/// `Accepted` confirms durable enqueue only, not that a turn/message now
/// exists (the driver may still be draining a running turn's inbox later).
/// `Rejected` is a normal outcome (the inbox gate, a scale safety valve), not
/// an error — handling it is the adaptor's own policy (surface it, or
/// silently drop + log).
#[derive(Debug, Clone)]
pub enum IngestOutcome {
    Accepted { strand_id: String },
    Rejected { reason: String },
}

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
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StrandRuntimeSnapshot {
    pub strand: Strand,
    pub messages: Vec<StrandMessage>,
    pub turns: Vec<Turn>,
    pub thinking_spans: Vec<ThinkingSpan>,
    pub tool_calls: Vec<ToolCall>,
    pub tool_results: Vec<ToolResult>,
    pub compacts: Vec<Compact>,
    pub effects: Vec<StrandEffect>,
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
