use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use crate::Timestamp;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = compact::Compact)]
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
#[schema(as = compact::Exec)]
pub struct Exec {
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
    pub capsule: Option<Capsule>,
    #[serde(default)]
    pub dry: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = compact::Capsule)]
pub struct Capsule {
    pub source: String,
    pub reason: String,
    pub risk: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queryability: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = compact::Report)]
pub struct Report {
    pub compact: String,
    pub first: String,
    pub last: String,
    pub from: i64,
    pub to: i64,
    pub absorbed: Vec<String>,
    pub collapsed: i64,
    #[serde(default)]
    pub dry: bool,
    #[serde(default)]
    pub active_incident_resolved: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<crate::budget::Estimate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<crate::budget::Estimate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ratio: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = compact::Entry)]
pub struct Entry {
    pub seq: i64,
    pub kind: crate::strand::Target,
    pub target: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = compact::Page)]
pub struct Page {
    pub compact: String,
    pub first: String,
    pub last: String,
    pub total: i64,
    pub page_index: i64,
    pub page_size: i64,
    pub entries: Vec<Entry>,
}
