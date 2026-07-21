use crate::{ErrorCategory, ErrorRetry, ErrorSeverity, ErrorTransitionKind, IncidentStatus};

pub fn category_db(value: ErrorCategory) -> &'static str {
    match value {
        ErrorCategory::Internal => "internal",
        ErrorCategory::InvalidInput => "invalid_input",
        ErrorCategory::NotFound => "not_found",
        ErrorCategory::ResourceExhausted => "resource_exhausted",
        ErrorCategory::Unauthorized => "unauthorized",
        ErrorCategory::Unavailable => "unavailable",
    }
}

pub fn category_from_db(value: &str) -> ErrorCategory {
    match value {
        "invalid_input" => ErrorCategory::InvalidInput,
        "not_found" => ErrorCategory::NotFound,
        "resource_exhausted" => ErrorCategory::ResourceExhausted,
        "unauthorized" => ErrorCategory::Unauthorized,
        "unavailable" => ErrorCategory::Unavailable,
        _ => ErrorCategory::Internal,
    }
}

pub fn severity_db(value: ErrorSeverity) -> &'static str {
    match value {
        ErrorSeverity::Error => "error",
    }
}

pub fn severity_from_db(_value: &str) -> ErrorSeverity {
    ErrorSeverity::Error
}

pub fn retry_db(value: ErrorRetry) -> &'static str {
    match value {
        ErrorRetry::Never => "never",
        ErrorRetry::Later => "later",
        ErrorRetry::AfterChange => "after_change",
        ErrorRetry::AfterResolution => "after_resolution",
    }
}

pub fn retry_from_db(value: &str) -> ErrorRetry {
    match value {
        "never" => ErrorRetry::Never,
        "after_change" => ErrorRetry::AfterChange,
        "after_resolution" => ErrorRetry::AfterResolution,
        _ => ErrorRetry::Later,
    }
}

pub fn incident_status_db(value: &IncidentStatus) -> &'static str {
    match value {
        IncidentStatus::Active => "active",
        IncidentStatus::Resolved => "resolved",
    }
}

pub fn incident_status_from_db(value: &str) -> IncidentStatus {
    match value {
        "resolved" => IncidentStatus::Resolved,
        _ => IncidentStatus::Active,
    }
}

pub fn transition_kind_db(value: &ErrorTransitionKind) -> &'static str {
    match value {
        ErrorTransitionKind::Opened => "opened",
        ErrorTransitionKind::Resolved => "resolved",
    }
}
