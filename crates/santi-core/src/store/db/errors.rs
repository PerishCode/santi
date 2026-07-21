use rusqlite::{OptionalExtension, Row, params};
use santi_error::{
    ErrorIncident, ErrorScope, ErrorSource, IncidentDraft, IncidentMutation, SantiError,
    category_db, category_from_db, engine, incident_status_db, incident_status_from_db, retry_db,
    retry_from_db, severity_db, severity_from_db, transition_kind_db,
};

use super::Database;

const INCIDENT_COLUMNS: &str = r#"
    id, incident_key, code, status, category, severity, retry, exposure,
    scope_kind, scope_id, source_component, source_operation,
    latest_source_component, latest_source_operation, message, latest_message,
    context, latest_context, occurrence_count, revision, first_seen_at,
    last_seen_at, resolved_at, resolved_by
"#;

impl Database<'_> {
    pub(crate) fn open_incident(&self, draft: IncidentDraft) -> Result<SantiError, String> {
        let existing = self.active_incident(&draft.incident_key)?;
        let mutation =
            engine().open_incident(existing.as_ref(), draft, santi_model::timestamp_now());
        self.persist_mutation(&mutation)?;
        Ok(mutation.error)
    }

    pub(crate) fn resolve_incident(
        &self,
        incident_key: &str,
        resolved_by: &str,
        context: serde_json::Value,
    ) -> Result<bool, String> {
        let Some(active) = self.active_incident(incident_key)? else {
            return Ok(false);
        };
        let mutation =
            engine().resolve_incident(&active, resolved_by, context, santi_model::timestamp_now());
        self.persist_mutation(&mutation)?;
        Ok(true)
    }

    pub(crate) fn active_incident(
        &self,
        incident_key: &str,
    ) -> Result<Option<ErrorIncident>, String> {
        self.conn
            .query_row(
                &format!(
                    "SELECT {INCIDENT_COLUMNS} FROM error_incidents WHERE incident_key = ?1 AND status = 'active' LIMIT 1"
                ),
                params![incident_key],
                map_incident_row,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub(crate) fn list_incidents(
        &self,
        scope_kind: &str,
        scope_id: &str,
        limit: i64,
    ) -> Result<Vec<ErrorIncident>, String> {
        let mut stmt = self
            .conn
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

    pub(crate) fn persist_mutation(&self, mutation: &IncidentMutation) -> Result<(), String> {
        let incident = &mutation.incident;
        self.conn
            .execute(
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
                    serde_json::to_string(&incident.latest_context)
                        .map_err(|error| error.to_string())?,
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
            self.conn
                .execute(
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
