use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::ErrorIncident;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct ContextEstimate {
    pub estimator: String,
    pub input_items: i64,
    pub input_bytes: i64,
    pub instructions_bytes: i64,
    pub tools_bytes: i64,
    pub total_bytes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct ContextBudget {
    pub input_budget_bytes: i64,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StrandBudgetSnapshot {
    pub strand_id: String,
    pub estimate: ContextEstimate,
    pub budget: Option<ContextBudget>,
    pub active_incident: Option<ErrorIncident>,
}
