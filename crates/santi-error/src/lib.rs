use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

pub mod catalog {
    use super::{ErrorCategory, ErrorDescriptor, ErrorExposure, ErrorRetry, ErrorSeverity};

    pub const CONTEXT_BUDGET_EXCEEDED: ErrorDescriptor = ErrorDescriptor {
        code: "context.budget.exceeded",
        category: ErrorCategory::ResourceExhausted,
        severity: ErrorSeverity::Error,
        retry: ErrorRetry::AfterResolution,
        exposure: ErrorExposure::CALLER_AND_OPERATOR,
    };

    pub const INBOX_CAPACITY_EXCEEDED: ErrorDescriptor = ErrorDescriptor {
        code: "runtime.inbox.capacity_exceeded",
        category: ErrorCategory::ResourceExhausted,
        severity: ErrorSeverity::Error,
        retry: ErrorRetry::Later,
        exposure: ErrorExposure::CALLER_AND_OPERATOR,
    };

    pub const EXECUTION_BUDGET_EXCEEDED: ErrorDescriptor = ErrorDescriptor {
        code: "runtime.execution_budget.exceeded",
        category: ErrorCategory::ResourceExhausted,
        severity: ErrorSeverity::Error,
        retry: ErrorRetry::AfterChange,
        exposure: ErrorExposure::CALLER_AND_OPERATOR,
    };

    pub const SOUL_MEMORY_INTERVENTION_REQUIRED: ErrorDescriptor = ErrorDescriptor {
        code: "runtime.soul_memory.intervention_required",
        category: ErrorCategory::ResourceExhausted,
        severity: ErrorSeverity::Error,
        retry: ErrorRetry::AfterChange,
        exposure: ErrorExposure::OPERATOR_ONLY,
    };

    pub const UPGRADE_FAILED: ErrorDescriptor = ErrorDescriptor {
        code: "runtime.upgrade.failed",
        category: ErrorCategory::Internal,
        severity: ErrorSeverity::Error,
        retry: ErrorRetry::AfterChange,
        exposure: ErrorExposure::OPERATOR_ONLY,
    };

    pub const UPGRADE_HANDOVER_FAILED: ErrorDescriptor = ErrorDescriptor {
        code: "runtime.upgrade.handover_failed",
        category: ErrorCategory::Internal,
        severity: ErrorSeverity::Error,
        retry: ErrorRetry::AfterChange,
        exposure: ErrorExposure::OPERATOR_ONLY,
    };

    pub const ERROR_ENGINE_PERSISTENCE_FAILED: ErrorDescriptor = ErrorDescriptor {
        code: "runtime.error_engine.persistence_failed",
        category: ErrorCategory::Internal,
        severity: ErrorSeverity::Error,
        retry: ErrorRetry::Later,
        exposure: ErrorExposure::OPERATOR_ONLY,
    };

    pub const PROVIDER_TURN_FAILED: ErrorDescriptor = ErrorDescriptor {
        code: "provider.turn.failed",
        category: ErrorCategory::Unavailable,
        severity: ErrorSeverity::Error,
        retry: ErrorRetry::Later,
        exposure: ErrorExposure::CALLER_AND_OPERATOR,
    };

    pub const RUNTIME_TURN_FAILED: ErrorDescriptor = ErrorDescriptor {
        code: "runtime.turn.failed",
        category: ErrorCategory::Internal,
        severity: ErrorSeverity::Error,
        retry: ErrorRetry::Later,
        exposure: ErrorExposure::CALLER_AND_OPERATOR,
    };

    pub const STRAND_DRIVE_FAILED: ErrorDescriptor = ErrorDescriptor {
        code: "runtime.strand.drive_failed",
        category: ErrorCategory::Unavailable,
        severity: ErrorSeverity::Error,
        retry: ErrorRetry::AfterResolution,
        exposure: ErrorExposure::CALLER_AND_OPERATOR,
    };

    pub const INVALID_ARGUMENT: ErrorDescriptor = ErrorDescriptor {
        code: "request.invalid_argument",
        category: ErrorCategory::InvalidInput,
        severity: ErrorSeverity::Error,
        retry: ErrorRetry::AfterChange,
        exposure: ErrorExposure::CALLER_AND_OPERATOR,
    };

    pub const NOT_FOUND: ErrorDescriptor = ErrorDescriptor {
        code: "resource.not_found",
        category: ErrorCategory::NotFound,
        severity: ErrorSeverity::Error,
        retry: ErrorRetry::AfterChange,
        exposure: ErrorExposure::CALLER_AND_OPERATOR,
    };

    pub const UNAUTHORIZED: ErrorDescriptor = ErrorDescriptor {
        code: "request.unauthorized",
        category: ErrorCategory::Unauthorized,
        severity: ErrorSeverity::Error,
        retry: ErrorRetry::AfterChange,
        exposure: ErrorExposure::CALLER_AND_OPERATOR,
    };

    pub const INTERNAL: ErrorDescriptor = ErrorDescriptor {
        code: "runtime.internal",
        category: ErrorCategory::Internal,
        severity: ErrorSeverity::Error,
        retry: ErrorRetry::Later,
        exposure: ErrorExposure::OPERATOR_ONLY,
    };
}

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

#[derive(Debug, Default)]
pub struct ErrorEngine;

static ENGINE: OnceLock<ErrorEngine> = OnceLock::new();

pub fn engine() -> &'static ErrorEngine {
    ENGINE.get_or_init(|| ErrorEngine)
}

impl ErrorEngine {
    pub fn open_incident(
        &self,
        existing: Option<&ErrorIncident>,
        draft: IncidentDraft,
        now: impl Into<String>,
    ) -> IncidentMutation {
        let now = now.into();
        let (incident, transition) = match existing {
            Some(existing) => {
                let mut incident = existing.clone();
                incident.latest_source = draft.source.clone();
                incident.latest_message = draft.message.clone();
                incident.latest_context = draft.context.clone();
                incident.occurrence_count += 1;
                incident.last_seen_at = now;
                (incident, None)
            }
            None => {
                let (incident, transition) = self.open_new(draft, now);
                (incident, Some(transition))
            }
        };
        let error = self.error_from_incident(&incident);
        IncidentMutation {
            incident,
            error,
            transition,
        }
    }

    pub fn resolve_incident(
        &self,
        active: &ErrorIncident,
        resolved_by: impl Into<String>,
        context: Value,
        now: impl Into<String>,
    ) -> IncidentMutation {
        let now = now.into();
        let mut incident = active.clone();
        incident.status = IncidentStatus::Resolved;
        incident.revision += 1;
        incident.last_seen_at = now.clone();
        incident.resolved_at = Some(now.clone());
        incident.resolved_by = Some(resolved_by.into());
        incident.latest_context = context;
        let transition = ErrorTransition {
            id: prefixed_id("error_event"),
            incident_id: incident.id.clone(),
            revision: incident.revision,
            kind: ErrorTransitionKind::Resolved,
            incident: incident.clone(),
            occurred_at: now,
        };
        let error = self.error_from_incident(&incident);
        IncidentMutation {
            incident,
            error,
            transition: Some(transition),
        }
    }

    pub fn transient(&self, signal: Signal) -> SantiError {
        SantiError {
            id: prefixed_id("error"),
            incident_id: None,
            code: signal.descriptor.code.to_string(),
            message: signal.message,
            category: signal.descriptor.category,
            severity: signal.descriptor.severity,
            retry: signal.descriptor.retry,
            exposure: signal.descriptor.exposure,
            source: signal.source,
            scope: signal.scope,
            context: signal.context,
        }
    }

    pub fn dispatch_outbox(
        &self,
        outbox: &impl ErrorOutbox,
        sink: &impl ErrorEventSink,
        limit: usize,
    ) -> Result<usize, String> {
        let transitions = outbox.pending_error_transitions(limit)?;
        let mut delivered = 0;
        for transition in transitions {
            sink.publish_error_transition(&transition)?;
            outbox.mark_error_transition_delivered(&transition.id)?;
            delivered += 1;
        }
        Ok(delivered)
    }

    fn error_from_incident(&self, incident: &ErrorIncident) -> SantiError {
        SantiError {
            id: prefixed_id("error"),
            incident_id: Some(incident.id.clone()),
            code: incident.code.clone(),
            message: incident.latest_message.clone(),
            category: incident.category,
            severity: incident.severity,
            retry: incident.retry,
            exposure: incident.exposure,
            source: incident.latest_source.clone(),
            scope: Some(incident.scope.clone()),
            context: incident.latest_context.clone(),
        }
    }

    fn open_new(&self, draft: IncidentDraft, now: String) -> (ErrorIncident, ErrorTransition) {
        let incident = ErrorIncident {
            id: prefixed_id("inc"),
            incident_key: draft.incident_key,
            code: draft.descriptor.code.to_string(),
            status: IncidentStatus::Active,
            category: draft.descriptor.category,
            severity: draft.descriptor.severity,
            retry: draft.descriptor.retry,
            exposure: draft.descriptor.exposure,
            scope: draft.scope,
            source: draft.source.clone(),
            latest_source: draft.source,
            message: draft.message.clone(),
            latest_message: draft.message,
            context: draft.context.clone(),
            latest_context: draft.context,
            occurrence_count: 1,
            revision: 1,
            first_seen_at: now.clone(),
            last_seen_at: now.clone(),
            resolved_at: None,
            resolved_by: None,
        };
        let transition = ErrorTransition {
            id: prefixed_id("error_event"),
            incident_id: incident.id.clone(),
            revision: incident.revision,
            kind: ErrorTransitionKind::Opened,
            incident: incident.clone(),
            occurred_at: now,
        };
        (incident, transition)
    }
}

fn prefixed_id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::new_v4().simple())
}
