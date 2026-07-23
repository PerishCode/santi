use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::Timestamp;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = downstream::Credential)]
pub struct Credential {
    pub id: String,
    pub prefix: String,
    #[serde(skip)]
    #[schema(ignore)]
    pub digest: String,
    pub created: Timestamp,
    pub updated: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = downstream::Draft)]
pub struct Draft {
    pub id: String,
    pub prefix: String,
    pub digest: String,
}
