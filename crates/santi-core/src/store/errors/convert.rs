use rusqlite::Row;
use santi_error::{
    ErrorCategory, ErrorIncident, ErrorRetry, ErrorScope, ErrorSeverity, ErrorSource,
    IncidentStatus,
};

pub(super) fn map_incident_row(row: &Row<'_>) -> rusqlite::Result<ErrorIncident> {
    let exposure = parse_json(row.get::<_, String>(7)?, 7)?;
    let context = parse_json(row.get::<_, String>(16)?, 16)?;
    let latest_context = parse_json(row.get::<_, String>(17)?, 17)?;
    Ok(ErrorIncident {
        id: row.get(0)?,
        incident_key: row.get(1)?,
        code: row.get(2)?,
        status: incident_status_from_db(&row.get::<_, String>(3)?),
        category: category_from_db(&row.get::<_, String>(4)?),
        severity: severity_from_db(&row.get::<_, String>(5)?),
        retry: retry_from_db(&row.get::<_, String>(6)?),
        exposure,
        scope: ErrorScope::new(row.get::<_, String>(8)?, row.get::<_, String>(9)?),
        source: ErrorSource::new(row.get::<_, String>(10)?, row.get::<_, String>(11)?),
        latest_source: ErrorSource::new(row.get::<_, String>(12)?, row.get::<_, String>(13)?),
        message: row.get(14)?,
        latest_message: row.get(15)?,
        context,
        latest_context,
        occurrence_count: row.get(18)?,
        revision: row.get(19)?,
        first_seen_at: row.get(20)?,
        last_seen_at: row.get(21)?,
        resolved_at: row.get(22)?,
        resolved_by: row.get(23)?,
    })
}

pub(super) fn parse_json<T: serde::de::DeserializeOwned>(
    raw: String,
    index: usize,
) -> rusqlite::Result<T> {
    serde_json::from_str(&raw).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

pub(super) fn category_db(value: ErrorCategory) -> &'static str {
    match value {
        ErrorCategory::Internal => "internal",
        ErrorCategory::InvalidInput => "invalid_input",
        ErrorCategory::NotFound => "not_found",
        ErrorCategory::ResourceExhausted => "resource_exhausted",
        ErrorCategory::Unauthorized => "unauthorized",
        ErrorCategory::Unavailable => "unavailable",
    }
}

pub(super) fn category_from_db(value: &str) -> ErrorCategory {
    match value {
        "invalid_input" => ErrorCategory::InvalidInput,
        "not_found" => ErrorCategory::NotFound,
        "resource_exhausted" => ErrorCategory::ResourceExhausted,
        "unauthorized" => ErrorCategory::Unauthorized,
        "unavailable" => ErrorCategory::Unavailable,
        _ => ErrorCategory::Internal,
    }
}

pub(super) fn severity_db(value: ErrorSeverity) -> &'static str {
    match value {
        ErrorSeverity::Error => "error",
    }
}

pub(super) fn severity_from_db(_value: &str) -> ErrorSeverity {
    ErrorSeverity::Error
}

pub(super) fn retry_db(value: ErrorRetry) -> &'static str {
    match value {
        ErrorRetry::Never => "never",
        ErrorRetry::Later => "later",
        ErrorRetry::AfterChange => "after_change",
        ErrorRetry::AfterResolution => "after_resolution",
    }
}

pub(super) fn retry_from_db(value: &str) -> ErrorRetry {
    match value {
        "never" => ErrorRetry::Never,
        "after_change" => ErrorRetry::AfterChange,
        "after_resolution" => ErrorRetry::AfterResolution,
        _ => ErrorRetry::Later,
    }
}

pub(super) fn incident_status_db(value: &IncidentStatus) -> &'static str {
    match value {
        IncidentStatus::Active => "active",
        IncidentStatus::Resolved => "resolved",
    }
}

pub(super) fn incident_status_from_db(value: &str) -> IncidentStatus {
    match value {
        "resolved" => IncidentStatus::Resolved,
        _ => IncidentStatus::Active,
    }
}

pub(super) fn transition_kind_db(value: &santi_error::ErrorTransitionKind) -> &'static str {
    match value {
        santi_error::ErrorTransitionKind::Opened => "opened",
        santi_error::ErrorTransitionKind::Resolved => "resolved",
    }
}
