use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::Timestamp;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[schema(as = trace::Tag)]
pub struct Tag {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = trace::Record)]
pub struct Record {
    pub name: String,
    pub tags: Vec<Tag>,
    pub opened: Timestamp,
    pub closed: Timestamp,
}
