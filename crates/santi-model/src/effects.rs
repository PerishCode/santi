use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::Timestamp;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EffectState {
    Prepared,
    Dispatching,
    Unknown,
    Confirmed,
    NotDispatched,
    ResolvedApplied,
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
    pub evidence: Option<String>,
    pub occurred: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EffectStatus {
    pub effect: StrandEffect,
    pub transitions: Vec<EffectTransition>,
    pub receipt_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EffectResolutionOutcome {
    Applied,
    NotApplied,
}
