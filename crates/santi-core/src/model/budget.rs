use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::Timestamp;

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
pub struct StrandBlock {
    pub id: String,
    pub strand_id: String,
    pub kind: String,
    pub status: String,
    pub reason_code: String,
    pub reason_text: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub budget_source: Option<String>,
    pub budget_bytes: Option<i64>,
    pub input_items: Option<i64>,
    pub input_bytes: Option<i64>,
    pub instructions_bytes: Option<i64>,
    pub tools_bytes: Option<i64>,
    pub total_bytes: Option<i64>,
    pub observed_turn_id: Option<String>,
    pub observed_at_seq: Option<i64>,
    pub metadata: Option<Value>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub cleared_at: Option<Timestamp>,
    pub cleared_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RejectedDelivery {
    pub id: String,
    pub strand_id: Option<String>,
    pub block_id: Option<String>,
    pub source_type: Option<String>,
    pub source_ref: Option<String>,
    pub source_metadata: Option<Value>,
    pub message_kind: Option<String>,
    pub content_sha256: String,
    pub content_bytes: i64,
    pub content_excerpt: String,
    pub reason_code: String,
    pub reason_text: String,
    pub received_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StrandBudgetSnapshot {
    pub strand_id: String,
    pub estimate: ContextEstimate,
    pub budget: Option<ContextBudget>,
    pub active_block: Option<StrandBlock>,
}
