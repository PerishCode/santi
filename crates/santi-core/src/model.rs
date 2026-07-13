use santi_error::SantiError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

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
    /// Aggregate only: `/health` is public and must never expose strand,
    /// receipt, or incident locators.
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
    pub created_at: Timestamp,
}

/// Provider provenance captured for a tool call so the call can be replayed
/// faithfully (the Responses adapter echoes the raw `item`). All optional —
/// chat_completions and older rows may have none.
#[derive(Debug, Clone, Default)]
pub struct ToolCallProvenance {
    /// The provider FAMILY this material belongs to (from `ProviderMetadata::
    /// provider`). Persisted so an adaptor can refuse to replay material minted
    /// for a different provider (PHASE-09 decision #9: unusable on mismatch).
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
pub struct ForkStrandResponse {
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
    pub receipt: IngestReceipt,
    pub turn: Option<Turn>,
    /// The content this send just enqueued, once the driver has actually
    /// committed it to the timeline. Absent when this send coalesced into an
    /// already-running turn — durably enqueued, but the driver has not drained
    /// it yet (it will, when that turn completes and re-pokes).
    pub user_message: Option<StrandMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IngestReceipt {
    pub strand_id: String,
    pub inbox_id: String,
    pub warning: Option<Box<SantiError>>,
}

/// Current durable responsibility state for one accepted inbox item. A
/// mechanically-recovered transition can be immediately followed by `driving`
/// in the same transaction; callers inspect `transitions` for that evidence.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptState {
    Accepted,
    MechanicallyRecovered,
    Driving,
    TurnFailed,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReceiptTransition {
    pub id: String,
    pub sequence: i64,
    pub state: ReceiptState,
    pub turn_id: Option<String>,
    pub incident_id: Option<String>,
    /// Present only when schema migration reconstructed this evidence from a
    /// durable v24 source row. Live transitions leave it unset.
    pub reconstructed_from: Option<String>,
    pub occurred_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReceiptStatus {
    pub inbox_id: String,
    pub strand_id: String,
    pub state: ReceiptState,
    pub accepted_at: Timestamp,
    pub updated_at: Timestamp,
    pub transitions: Vec<ReceiptTransition>,
    /// Per-attempt external effects reached by any turn carrying this receipt.
    /// Receipt completion remains assistant-turn persistence only.
    pub effects: Vec<StrandEffect>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DriveStrandState {
    Started,
    Running,
    Idle,
    Paused,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DriveStrandResponse {
    pub strand_id: String,
    pub state: DriveStrandState,
    pub turn: Option<Turn>,
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
/// `Rejected` is a quick-fail boundary result. The canonical error carries the
/// incident identity when durable operator intervention is required.
#[derive(Debug, Clone)]
pub enum IngestOutcome {
    Accepted { receipt: IngestReceipt },
    Rejected { error: Box<SantiError> },
}

/// Bounded provenance for an inbound item at the moment it is enqueued. This is
/// runtime evidence, not model-visible message content: provider assembly reads
/// `messages`, while this metadata is carried only into drain/audit diagnostics.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InboxSource {
    pub source_type: String,
    pub source_ref: Option<String>,
    pub metadata: Option<Value>,
}

impl InboxSource {
    pub fn new(source_type: impl Into<String>) -> Self {
        Self {
            source_type: source_type.into(),
            source_ref: None,
            metadata: None,
        }
    }

    pub fn with_ref(mut self, source_ref: impl Into<String>) -> Self {
        self.source_ref = Some(source_ref.into());
        self
    }

    pub fn with_metadata(mut self, metadata: Value) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

/// The external-label prefix that marks a strand as an IM conversation. The IM
/// layer builds `im:<participant_id>` labels; the reply-routing correlation
/// strips this back to the participant. Shared by the IM store, the API send
/// handler, and the offline `im reply` egress.
pub const IM_LABEL_PREFIX: &str = "im:";

/// IM inbound: a participant sends `content` to the soul `soul_id`. The runtime
/// primitive stays source-less — the sender's address (`participant_id`) is IM
/// envelope only, carried into the `im:<participant_id>` conversation label.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImSendRequest {
    pub soul_id: String,
    pub participant_id: String,
    pub content: String,
}

/// Result of an IM send: durable-enqueue confirmation (the soul may still be
/// mid-turn). Poll `GET /api/v1/im/inbox/{participant_id}` for the reply.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImSendResponse {
    pub participant_id: String,
    pub receipt: IngestReceipt,
}

/// A persistent IM participant — a messaging endpoint in the plain IM integrated
/// into santi (conceptually orthogonal to the runtime). A `human` participant has
/// a passive inbox (polled); a `soul` participant's inbox is its strand and is
/// NOT stored in the IM tables.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImParticipant {
    pub id: String,
    pub kind: String,
    pub created_at: Timestamp,
}

/// One delivered message in a participant's IM inbox — a return value it catches.
/// `seq` is a global monotonic cursor (the caller polls `seq > since` and dedups
/// by it; the high-water `seq` IS the ack). `from_ref` names the soul strand that
/// replied.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImInboxEntry {
    pub seq: i64,
    pub id: String,
    pub participant_id: String,
    pub from_ref: Option<String>,
    pub content: String,
    pub created_at: Timestamp,
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
