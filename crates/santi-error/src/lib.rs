use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

pub mod catalog;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    Internal,
    InvalidInput,
    NotFound,
    ResourceExhausted,
    Unauthorized,
    Unavailable,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorSeverity {
    Error,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorRetry {
    Never,
    Later,
    AfterChange,
    AfterResolution,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct ErrorExposure {
    pub caller: bool,
    pub operator: bool,
    pub model: bool,
}

impl ErrorExposure {
    pub const CALLER_AND_OPERATOR: Self = Self {
        caller: true,
        operator: true,
        model: false,
    };
    pub const OPERATOR_ONLY: Self = Self {
        caller: false,
        operator: true,
        model: false,
    };
}

#[derive(Debug, Clone, Copy)]
pub struct ErrorDescriptor {
    pub code: &'static str,
    pub category: ErrorCategory,
    pub severity: ErrorSeverity,
    pub retry: ErrorRetry,
    pub exposure: ErrorExposure,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct ErrorScope {
    pub kind: String,
    pub id: String,
}

impl ErrorScope {
    pub fn new(kind: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            id: id.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct ErrorSource {
    pub component: String,
    pub operation: String,
}

impl ErrorSource {
    pub fn new(component: impl Into<String>, operation: impl Into<String>) -> Self {
        Self {
            component: component.into(),
            operation: operation.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IncidentStatus {
    Active,
    Resolved,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ErrorIncident {
    pub id: String,
    pub incident_key: String,
    pub code: String,
    pub status: IncidentStatus,
    pub category: ErrorCategory,
    pub severity: ErrorSeverity,
    pub retry: ErrorRetry,
    pub exposure: ErrorExposure,
    pub scope: ErrorScope,
    pub source: ErrorSource,
    pub latest_source: ErrorSource,
    pub message: String,
    pub latest_message: String,
    pub context: Value,
    pub latest_context: Value,
    pub occurrence_count: i64,
    pub revision: i64,
    pub first_seen_at: String,
    pub last_seen_at: String,
    pub resolved_at: Option<String>,
    pub resolved_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SantiError {
    pub id: String,
    pub incident_id: Option<String>,
    pub code: String,
    pub message: String,
    pub category: ErrorCategory,
    pub severity: ErrorSeverity,
    pub retry: ErrorRetry,
    pub exposure: ErrorExposure,
    pub source: ErrorSource,
    pub scope: Option<ErrorScope>,
    pub context: Value,
}

impl std::fmt::Display for SantiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)?;
        if let Some(incident_id) = self.incident_id.as_deref() {
            write!(formatter, " (incident {incident_id})")?;
        }
        Ok(())
    }
}

impl std::error::Error for SantiError {}

#[derive(Debug, Clone)]
pub struct IncidentDraft {
    pub incident_key: String,
    pub descriptor: ErrorDescriptor,
    pub scope: ErrorScope,
    pub source: ErrorSource,
    pub message: String,
    pub context: Value,
}

#[derive(Debug, Clone)]
pub struct Signal {
    pub descriptor: ErrorDescriptor,
    pub source: ErrorSource,
    pub scope: Option<ErrorScope>,
    pub message: String,
    pub context: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorTransitionKind {
    Opened,
    Resolved,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ErrorTransition {
    pub id: String,
    pub incident_id: String,
    pub revision: i64,
    pub kind: ErrorTransitionKind,
    pub incident: ErrorIncident,
    pub occurred_at: String,
}

#[derive(Debug, Clone)]
pub struct IncidentMutation {
    pub incident: ErrorIncident,
    pub error: SantiError,
    pub transition: Option<ErrorTransition>,
}

pub trait ErrorOutbox {
    fn pending_error_transitions(&self, limit: usize) -> Result<Vec<ErrorTransition>, String>;
    fn mark_error_transition_delivered(&self, transition_id: &str) -> Result<(), String>;
}

pub trait ErrorEventSink {
    fn publish_error_transition(&self, transition: &ErrorTransition) -> Result<(), String>;
}

mod engine;
pub use engine::*;
