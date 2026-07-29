use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::Timestamp;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = environ::Scope)]
pub enum Scope {
    Soul,
    Strand,
}

impl Scope {
    pub fn encode(self) -> &'static str {
        match self {
            Self::Soul => "soul",
            Self::Strand => "strand",
        }
    }

    pub fn unit(self) -> &'static str {
        match self {
            Self::Soul => "Soul",
            Self::Strand => "Strand",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = environ::Variable)]
pub struct Variable {
    pub scope: Scope,
    pub owner: String,
    pub name: String,
    pub value: String,
    pub created: Timestamp,
    pub updated: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = environ::Draft)]
pub struct Draft {
    pub name: String,
    pub value: String,
}
