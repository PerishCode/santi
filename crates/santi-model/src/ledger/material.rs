use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::Timestamp;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
#[schema(as = material::Kind)]
pub enum Kind {
    SystemPrompt,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = material::Request)]
pub struct Request {
    pub kind: Kind,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = material::Material)]
pub struct Material {
    pub strand: String,
    pub kind: Kind,
    pub content_type: String,
    pub text: String,
    pub updated: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = material::Updated)]
pub struct Updated {
    pub strand: String,
    pub kind: Kind,
    pub updated: Timestamp,
}
