use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::Timestamp;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = soul::Soul)]
pub struct Soul {
    pub id: String,
    pub created: Timestamp,
    pub updated: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = soul::Draft)]
pub struct Draft {
    #[serde(default)]
    pub memory: Option<String>,
}
