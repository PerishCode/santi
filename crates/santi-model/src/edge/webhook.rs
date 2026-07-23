use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::Timestamp;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = webhook::Subscription)]
pub struct Subscription {
    pub name: String,
    pub adaptor: String,
    pub soul: String,
    pub strategy: String,
    pub credential: String,
    pub created: Timestamp,
    pub updated: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = webhook::Draft)]
pub struct Draft {
    pub name: String,
    pub adaptor: String,
    pub soul: String,
    #[serde(default)]
    pub strategy: Option<String>,
    pub credential: String,
}
