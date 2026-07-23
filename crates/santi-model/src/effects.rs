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
    pub strand: String,
    pub turn: String,
    pub call: Option<String>,
    pub kind: String,
    pub state: EffectState,
    pub result: Option<String>,
    pub error: Option<String>,
    pub created: Timestamp,
    pub updated: Timestamp,
    pub dispatched: Option<Timestamp>,
    pub settled: Option<Timestamp>,
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
    pub receipts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EffectResolutionOutcome {
    Applied,
    NotApplied,
}
