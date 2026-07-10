use rusqlite::{Connection, OptionalExtension, Row, params};
use santi_error::{
    ErrorCategory, ErrorIncident, ErrorOutbox, ErrorRetry, ErrorScope, ErrorSeverity, ErrorSource,
    ErrorTransition, IncidentDraft, IncidentMutation, IncidentStatus, SantiError, engine,
};

use super::SantiStore;
use crate::timestamp_now;

const INCIDENT_COLUMNS: &str = r#"
    id, incident_key, code, status, category, severity, retry, exposure,
    scope_kind, scope_id, source_component, source_operation,
    latest_source_component, latest_source_operation, message, latest_message,
    context, latest_context, occurrence_count, revision, first_seen_at,
    last_seen_at, resolved_at, resolved_by
"#;

pub(super) fn open_incident_in_conn(
    conn: &Connection,
    draft: IncidentDraft,
) -> Result<SantiError, String> {
    let existing = active_in_conn(conn, &draft.incident_key)?;
    let mutation = engine().open_incident(existing.as_ref(), draft, timestamp_now());
    persist_mutation(conn, &mutation)?;
    Ok(mutation.error)
}

pub(super) fn resolve_in_conn(
    conn: &Connection,
    incident_key: &str,
    resolved_by: &str,
    context: serde_json::Value,
) -> Result<bool, String> {
    let Some(active) = active_in_conn(conn, incident_key)? else {
        return Ok(false);
    };
    let mutation = engine().resolve_incident(&active, resolved_by, context, timestamp_now());
    persist_mutation(conn, &mutation)?;
    Ok(true)
}

pub(super) fn active_in_conn(
    conn: &Connection,
    incident_key: &str,
) -> Result<Option<ErrorIncident>, String> {
    conn.query_row(
        &format!(
            "SELECT {INCIDENT_COLUMNS} FROM error_incidents WHERE incident_key = ?1 AND status = 'active' LIMIT 1"
        ),
        params![incident_key],
        map_incident_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

pub(super) fn list_in_conn(
    conn: &Connection,
    scope_kind: &str,
    scope_id: &str,
    limit: i64,
) -> Result<Vec<ErrorIncident>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {INCIDENT_COLUMNS} FROM error_incidents \
             WHERE scope_kind = ?1 AND scope_id = ?2 \
             ORDER BY first_seen_at DESC, id DESC LIMIT ?3"
        ))
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(
            params![scope_kind, scope_id, limit.clamp(1, 1000)],
            map_incident_row,
        )
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

impl SantiStore {
    pub fn open_error_incident(&self, draft: IncidentDraft) -> Result<SantiError, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let error = open_incident_in_conn(&tx, draft)?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(error)
    }

    pub fn resolve_error_incident(
        &self,
        incident_key: &str,
        resolved_by: &str,
        context: serde_json::Value,
    ) -> Result<bool, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let resolved = resolve_in_conn(&tx, incident_key, resolved_by, context)?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(resolved)
    }

    pub fn error_incidents(
        &self,
        scope: &ErrorScope,
        limit: i64,
    ) -> Result<Vec<ErrorIncident>, String> {
        let conn = self.conn.lock().unwrap();
        list_in_conn(&conn, &scope.kind, &scope.id, limit)
    }

    pub(crate) fn active_error_incident(
        &self,
        incident_key: &str,
    ) -> Result<Option<ErrorIncident>, String> {
        let conn = self.conn.lock().unwrap();
        active_in_conn(&conn, incident_key)
    }

    pub(crate) fn error_incidents_for_strand(
        &self,
        strand_id: &str,
        limit: i64,
    ) -> Result<Vec<ErrorIncident>, String> {
        self.error_incidents(&ErrorScope::new("strand", strand_id), limit)
    }
}

impl ErrorOutbox for SantiStore {
    fn pending_error_transitions(&self, limit: usize) -> Result<Vec<ErrorTransition>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                r#"
                SELECT payload
                FROM error_transitions
                WHERE delivered_at IS NULL
                ORDER BY created_at ASC, id ASC
                LIMIT ?1
                "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![limit.min(i64::MAX as usize) as i64], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|error| error.to_string())?;
        let mut transitions = Vec::new();
        for row in rows {
            let payload = row.map_err(|error| error.to_string())?;
            transitions.push(serde_json::from_str(&payload).map_err(|error| error.to_string())?);
        }
        Ok(transitions)
    }

    fn mark_error_transition_delivered(&self, transition_id: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE error_transitions SET delivered_at = ?2 WHERE id = ?1 AND delivered_at IS NULL",
            params![transition_id, timestamp_now()],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    }
}

fn persist_mutation(conn: &Connection, mutation: &IncidentMutation) -> Result<(), String> {
    let incident = &mutation.incident;
    conn.execute(
        r#"
        INSERT INTO error_incidents (
          id, incident_key, code, status, category, severity, retry, exposure,
          scope_kind, scope_id, source_component, source_operation,
          latest_source_component, latest_source_operation, message, latest_message,
          context, latest_context, occurrence_count, revision, first_seen_at,
          last_seen_at, resolved_at, resolved_by
        ) VALUES (
          ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
          ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24
        )
        ON CONFLICT(id) DO UPDATE SET
          status = excluded.status,
          latest_source_component = excluded.latest_source_component,
          latest_source_operation = excluded.latest_source_operation,
          latest_message = excluded.latest_message,
          latest_context = excluded.latest_context,
          occurrence_count = excluded.occurrence_count,
          revision = excluded.revision,
          last_seen_at = excluded.last_seen_at,
          resolved_at = excluded.resolved_at,
          resolved_by = excluded.resolved_by
        "#,
        params![
            incident.id,
            incident.incident_key,
            incident.code,
            incident_status_db(&incident.status),
            category_db(incident.category),
            severity_db(incident.severity),
            retry_db(incident.retry),
            serde_json::to_string(&incident.exposure).map_err(|error| error.to_string())?,
            incident.scope.kind,
            incident.scope.id,
            incident.source.component,
            incident.source.operation,
            incident.latest_source.component,
            incident.latest_source.operation,
            incident.message,
            incident.latest_message,
            serde_json::to_string(&incident.context).map_err(|error| error.to_string())?,
            serde_json::to_string(&incident.latest_context).map_err(|error| error.to_string())?,
            incident.occurrence_count,
            incident.revision,
            incident.first_seen_at,
            incident.last_seen_at,
            incident.resolved_at,
            incident.resolved_by,
        ],
    )
    .map_err(|error| error.to_string())?;

    if let Some(transition) = mutation.transition.as_ref() {
        conn.execute(
            r#"
            INSERT INTO error_transitions (
              id, incident_id, revision, kind, payload, created_at, delivered_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)
            "#,
            params![
                transition.id,
                transition.incident_id,
                transition.revision,
                transition_kind_db(&transition.kind),
                serde_json::to_string(transition).map_err(|error| error.to_string())?,
                transition.occurred_at,
            ],
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn map_incident_row(row: &Row<'_>) -> rusqlite::Result<ErrorIncident> {
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

fn parse_json<T: serde::de::DeserializeOwned>(raw: String, index: usize) -> rusqlite::Result<T> {
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
    }
}

fn category_from_db(value: &str) -> ErrorCategory {
    match value {
        "invalid_input" => ErrorCategory::InvalidInput,
        "not_found" => ErrorCategory::NotFound,
        "resource_exhausted" => ErrorCategory::ResourceExhausted,
        "unauthorized" => ErrorCategory::Unauthorized,
        _ => ErrorCategory::Internal,
    }
}

pub(super) fn severity_db(value: ErrorSeverity) -> &'static str {
    match value {
        ErrorSeverity::Error => "error",
    }
}

fn severity_from_db(_value: &str) -> ErrorSeverity {
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

fn retry_from_db(value: &str) -> ErrorRetry {
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

fn incident_status_from_db(value: &str) -> IncidentStatus {
    match value {
        "resolved" => IncidentStatus::Resolved,
        _ => IncidentStatus::Active,
    }
}

fn transition_kind_db(value: &santi_error::ErrorTransitionKind) -> &'static str {
    match value {
        santi_error::ErrorTransitionKind::Opened => "opened",
        santi_error::ErrorTransitionKind::Resolved => "resolved",
    }
}
