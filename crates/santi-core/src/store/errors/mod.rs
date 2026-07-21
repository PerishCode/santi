use rusqlite::{OptionalExtension, params};
use santi_error::{
    ErrorIncident, ErrorOutbox, ErrorScope, ErrorTransition, IncidentDraft, IncidentMutation,
    SantiError, engine,
};

use super::{SantiStore, db::Database};
use crate::timestamp_now;

mod convert;
use convert::*;
use santi_error::{category_db, incident_status_db, retry_db, severity_db, transition_kind_db};

pub(crate) mod drive;

const INCIDENT_COLUMNS: &str = r#"
    id, incident_key, code, status, category, severity, retry, exposure,
    scope_kind, scope_id, source_component, source_operation,
    latest_source_component, latest_source_operation, message, latest_message,
    context, latest_context, occurrence_count, revision, first_seen_at,
    last_seen_at, resolved_at, resolved_by
"#;

impl Database<'_> {
    pub(in crate::store) fn open_incident(
        &self,
        draft: IncidentDraft,
    ) -> Result<SantiError, String> {
        let existing = self.active_incident(&draft.incident_key)?;
        let mutation = engine().open_incident(existing.as_ref(), draft, timestamp_now());
        self.persist_mutation(&mutation)?;
        Ok(mutation.error)
    }

    pub(in crate::store) fn resolve_incident(
        &self,
        incident_key: &str,
        resolved_by: &str,
        context: serde_json::Value,
    ) -> Result<bool, String> {
        let Some(active) = self.active_incident(incident_key)? else {
            return Ok(false);
        };
        let mutation = engine().resolve_incident(&active, resolved_by, context, timestamp_now());
        self.persist_mutation(&mutation)?;
        Ok(true)
    }

    pub(in crate::store) fn active_incident(
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

    pub(in crate::store) fn list_incidents(
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
}

impl SantiStore {
    pub fn open_error_incident(&self, draft: IncidentDraft) -> Result<SantiError, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let error = Database::new(&tx).open_incident(draft)?;
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
        let resolved = Database::new(&tx).resolve_incident(incident_key, resolved_by, context)?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(resolved)
    }

    pub fn error_incidents(
        &self,
        scope: &ErrorScope,
        limit: i64,
    ) -> Result<Vec<ErrorIncident>, String> {
        let conn = self.conn.lock().unwrap();
        Database::new(&conn).list_incidents(&scope.kind, &scope.id, limit)
    }

    pub(crate) fn active_error_incident(
        &self,
        incident_key: &str,
    ) -> Result<Option<ErrorIncident>, String> {
        let conn = self.conn.lock().unwrap();
        Database::new(&conn).active_incident(incident_key)
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

impl Database<'_> {
    fn persist_mutation(&self, mutation: &IncidentMutation) -> Result<(), String> {
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
