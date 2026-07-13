use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::Timestamp;

/// Durable truth for one concrete external-effect attempt. It is deliberately
/// not turn state: one turn may contain several independently settled or
/// ambiguous effects.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EffectState {
    /// Intent and tool occurrence are durable; dispatch has not begun.
    Prepared,
    /// The dispatch ambiguity window is open. The process may already exist.
    Dispatching,
    /// Restart/result-capture evidence cannot prove whether the effect applied.
    Unknown,
    /// The runtime durably captured the process result.
    Confirmed,
    /// Process creation was mechanically rejected before the command ran.
    NotDispatched,
    /// An operator supplied evidence that an ambiguous effect applied.
    ResolvedApplied,
    /// An operator supplied evidence that an ambiguous effect did not apply.
    ResolvedNotApplied,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EffectTransitionReason {
    IntentPersisted,
    DispatchWindowOpened,
    ResultPersisted,
    DispatchRejected,
    RestartBeforeDispatch,
    RestartDuringDispatch,
    TurnFailedBeforeDispatch,
    TurnFailedDuringDispatch,
    ResultCaptureFailed,
    OperatorResolvedApplied,
    OperatorResolvedNotApplied,
    LegacyImport,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StrandEffect {
    pub id: String,
    pub strand_id: String,
    pub turn_id: String,
    /// Absent only for an imported legacy row whose old schema had no neutral
    /// tool-call locator.
    pub tool_call_id: Option<String>,
    pub effect_type: String,
    pub state: EffectState,
    pub result_ref: Option<String>,
    pub error_text: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub dispatched_at: Option<Timestamp>,
    pub settled_at: Option<Timestamp>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EffectTransition {
    pub id: String,
    pub sequence: i64,
    pub state: EffectState,
    pub reason: EffectTransitionReason,
    /// Human- or runtime-supplied evidence. This is never interpreted as proof
    /// of idempotency by core.
    pub evidence: Option<String>,
    pub occurred_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EffectStatus {
    pub effect: StrandEffect,
    pub transitions: Vec<EffectTransition>,
    /// Obligation roots whose attempts include this effect's turn.
    pub receipt_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EffectResolutionOutcome {
    Applied,
    NotApplied,
}
