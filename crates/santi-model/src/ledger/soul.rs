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

#[derive(Debug, Clone, Copy)]
pub enum Error {
    Intervention,
}

impl santi_error::Ruled for Error {
    fn descriptor(&self) -> santi_error::Descriptor {
        use santi_error::{Category, Exposure, Retry, Severity};
        match self {
            Self::Intervention => santi_error::Descriptor {
                code: "runtime.soul_memory.intervention_required",
                category: Category::Exhausted,
                severity: Severity::Error,
                retry: Retry::Changed,
                exposure: Exposure::OPERATOR_ONLY,
            },
        }
    }
}
