use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::{ContextEstimate, StrandTargetType, Timestamp};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Compact {
    pub id: String,
    pub strand: String,
    pub summary: String,
    pub first: String,
    pub last: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CompactExecRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<i64>,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capsule: Option<CompactCapsuleOptions>,
    #[serde(default)]
    pub dry: bool,
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
    pub compact: String,
    pub first: String,
    pub last: String,
    pub start_seq: i64,
    pub end_seq: i64,
    pub absorbed: Vec<String>,
    pub collapsed_count: i64,
    #[serde(default)]
    pub dry: bool,
    #[serde(default)]
    pub active_incident_resolved: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<ContextEstimate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<ContextEstimate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ratio: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CompactQueryEntry {
    pub seq: i64,
    pub kind: StrandTargetType,
    pub target: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CompactQueryResponse {
    pub compact: String,
    pub first: String,
    pub last: String,
    pub total: i64,
    pub page_index: i64,
    pub page_size: i64,
    pub entries: Vec<CompactQueryEntry>,
}
