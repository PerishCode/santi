use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use crate::Timestamp;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = tool::Call)]
pub struct Call {
    pub id: String,
    pub turn: String,
    pub tool: String,
    pub arguments: Value,
    pub created: Timestamp,
}

#[derive(Debug, Clone, Default)]
pub struct Provenance {
    pub family: String,
    pub item: Option<Value>,
    pub mark: Option<String>,
    pub response: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = tool::Reply)]
pub struct Reply {
    pub id: String,
    pub call: String,
    pub output: Option<Value>,
    pub error: Option<String>,
    pub created: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "beat", rename_all = "snake_case")]
#[schema(as = tool::Beat)]
pub enum Beat {
    Called { call: Call },
    Replied { result: Reply },
}
