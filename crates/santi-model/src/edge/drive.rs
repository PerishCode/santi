use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[schema(as = drive::State)]
pub enum State {
    Started,
    Running,
    Idle,
    Paused,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = drive::Response)]
pub struct Response {
    pub strand: String,
    pub state: State,
    pub turn: Option<crate::turn::Turn>,
}

#[derive(Debug, Clone, Copy)]
pub enum Error {
    Failed,
}

impl santi_error::Ruled for Error {
    fn descriptor(&self) -> santi_error::Descriptor {
        use santi_error::{Category, Exposure, Retry, Severity};
        match self {
            Self::Failed => santi_error::Descriptor {
                code: "runtime.strand.drive_failed",
                category: Category::Unavailable,
                severity: Severity::Error,
                retry: Retry::Resolved,
                exposure: Exposure::CALLER_AND_OPERATOR,
            },
        }
    }
}
