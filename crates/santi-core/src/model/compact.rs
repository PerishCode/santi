use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::{ContextEstimate, StrandTargetType, Timestamp};

/// A compact is a pure projection overlay over a strand's spine. It
/// self-describes its coverage by message-id boundaries and carries the
/// operator-authored summary while originals remain queryable.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Compact {
    pub id: String,
    pub strand_id: String,
    pub summary: String,
    pub start_message_id: String,
    pub end_message_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CompactExecRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_seq: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_seq: Option<i64>,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capsule: Option<CompactCapsuleOptions>,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CompactCapsuleOptions {
    pub source: String,
    pub reason: String,
    pub risk: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queryability: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CompactExecResponse {
    pub compact_id: String,
    pub start_message_id: String,
    pub end_message_id: String,
    pub start_seq: i64,
    pub end_seq: i64,
    pub absorbed: Vec<String>,
    pub collapsed_count: i64,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub active_block_cleared: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_estimate: Option<ContextEstimate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_estimate: Option<ContextEstimate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compression_ratio: Option<f64>,
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
