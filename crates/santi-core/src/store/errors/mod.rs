use rusqlite::params;
use santi_error::{
    ErrorIncident, ErrorOutbox, ErrorScope, ErrorTransition, IncidentDraft, SantiError,
};

use super::{SantiStore, db::Database};
use crate::timestamp_now;

pub(crate) mod drive;

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
