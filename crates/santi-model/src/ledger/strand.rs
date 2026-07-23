use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use crate::Timestamp;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = strand::Strand)]
pub struct Strand {
    pub id: String,
    pub soul: String,
    pub label: Option<String>,
    pub memory: String,
    pub state: Option<Value>,
    pub next: i64,
    pub seen: i64,
    pub parent: Option<String>,
    pub fork: Option<i64>,
    pub created: Timestamp,
    pub updated: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[schema(as = strand::Target)]
pub enum Target {
    Message,
    Compact,
    Thinking,
    ToolCall,
    ToolResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = strand::Entry)]
pub struct Entry {
    pub strand: String,
    pub kind: Target,
    pub target: String,
    pub seq: i64,
    pub created: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = strand::Created)]
pub struct Created {
    pub strand: Strand,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = strand::Forked)]
pub struct Forked {
    pub strand: Strand,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = strand::Detail)]
pub struct Detail {
    pub strand: Strand,
    pub messages: Vec<crate::message::Placed>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = strand::Post)]
pub struct Post {
    pub content: Vec<crate::message::Part>,
}

impl Post {
    pub fn text(&self) -> String {
        crate::message::Content {
            parts: self.content.clone(),
        }
        .rendered()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = strand::Posted)]
pub struct Posted {
    pub strand: Strand,
    pub receipt: crate::ingest::Receipt,
    pub turn: Option<crate::turn::Turn>,
    pub message: Option<crate::message::Placed>,
}

#[derive(Debug, Clone)]
pub enum Selector {
    ById(String),
    ByLabel { soul: String, label: String },
}
