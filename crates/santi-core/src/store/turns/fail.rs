use crate::store::{ProviderFault, RuntimeFault, SantiStore, db::Database};
use crate::{
    EffectTransitionReason, ErrorScope, ErrorSource, IncidentDraft, SantiError, Turn, catalog,
    timestamp_now,
};
use rusqlite::params;
use serde_json::json;

use super::*;

impl SantiStore {
    pub(crate) fn fail_provider_turn(
        &self,
        turn_id: &str,
        error_text: &str,
        failure: ProviderFault<'_>,
    ) -> Result<(Turn, SantiError), String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let strand_id: String = tx
            .query_row(
                "SELECT strand_id FROM turns WHERE id = ?1",
                params![turn_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let now = timestamp_now();
        tx.execute(
            r#"
            UPDATE turns
            SET status = 'failed', error_text = ?2, updated_at = ?3, finished_at = ?3
            WHERE id = ?1
            "#,
            params![turn_id, error_text, now],
        )
        .map_err(|error| error.to_string())?;
        let error = Database::new(&tx).open_incident(IncidentDraft {
            incident_key: provider_incident_key(&strand_id),
            descriptor: catalog::PROVIDER_TURN_FAILED,
            scope: ErrorScope::new("strand", &strand_id),
            source: ErrorSource::new("santi-provider", failure.operation),
            message: format!("provider {} failed", failure.stage),
            context: json!({
                "turn_id": turn_id,
                "provider": failure.provider,
                "model": failure.model,
                "stage": failure.stage,
                "round": failure.round,
                "detail": bounded_provider_detail(failure.detail),
                "trace": format!("log://turn/{turn_id}"),
            }),
        })?;
        Database::new(&tx).reconcile_effects(
            turn_id,
            EffectTransitionReason::TurnFailedBeforeDispatch,
            EffectTransitionReason::TurnFailedDuringDispatch,
            &now,
        )?;
        Database::new(&tx).fail_turn(turn_id, error.incident_id.as_deref(), &now)?;
        tx.commit().map_err(|error| error.to_string())?;
        let turn = Database::new(&conn)
            .turn_by_id(turn_id)?
            .ok_or_else(|| "failed turn missing".to_string())?;
        Ok((turn, error))
    }

    pub(crate) fn fail_runtime_turn(
        &self,
        turn_id: &str,
        error_text: &str,
        failure: RuntimeFault<'_>,
    ) -> Result<(Turn, SantiError), String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let strand_id: String = tx
            .query_row(
                "SELECT strand_id FROM turns WHERE id = ?1",
                params![turn_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let now = timestamp_now();
        tx.execute(
            r#"
            UPDATE turns
            SET status = 'failed', error_text = ?2, updated_at = ?3, finished_at = ?3
            WHERE id = ?1
            "#,
            params![turn_id, error_text, now],
        )
        .map_err(|error| error.to_string())?;
        let error = open_runtime_incident(&tx, &strand_id, turn_id, failure)?;
        Database::new(&tx).reconcile_effects(
            turn_id,
            EffectTransitionReason::TurnFailedBeforeDispatch,
            EffectTransitionReason::TurnFailedDuringDispatch,
            &now,
        )?;
        Database::new(&tx).fail_turn(turn_id, error.incident_id.as_deref(), &now)?;
        tx.commit().map_err(|error| error.to_string())?;
        let turn = Database::new(&conn)
            .turn_by_id(turn_id)?
            .ok_or_else(|| "failed turn missing".to_string())?;
        Ok((turn, error))
    }

    pub fn fail_turn(&self, turn_id: &str, error_text: &str) -> Result<Turn, String> {
        self.fail_turn_with_incident(turn_id, error_text, None)
    }

    pub(crate) fn fail_turn_with_incident(
        &self,
        turn_id: &str,
        error_text: &str,
        incident_id: Option<&str>,
    ) -> Result<Turn, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let now = timestamp_now();
        tx.execute(
            r#"
            UPDATE turns
            SET status = 'failed', error_text = ?2, updated_at = ?3, finished_at = ?3
            WHERE id = ?1
            "#,
            params![turn_id, error_text, now],
        )
        .map_err(|error| error.to_string())?;
        Database::new(&tx).reconcile_effects(
            turn_id,
            EffectTransitionReason::TurnFailedBeforeDispatch,
            EffectTransitionReason::TurnFailedDuringDispatch,
            &now,
        )?;
        Database::new(&tx).fail_turn(turn_id, incident_id, &now)?;
        tx.commit().map_err(|error| error.to_string())?;
        Database::new(&conn)
            .turn_by_id(turn_id)?
            .ok_or_else(|| "failed turn missing".to_string())
    }

    pub fn finish_failed_turn_context(
        &self,
        turn_id: &str,
        last_seen_strand_seq: i64,
    ) -> Result<Turn, String> {
        let conn = self.conn.lock().unwrap();
        let now = timestamp_now();
        conn.execute(
            r#"
            UPDATE turns
            SET end_strand_seq = (
                  SELECT next_seq - 1 FROM strands WHERE id = turns.strand_id
                ),
                updated_at = ?2
            WHERE id = ?1 AND status = 'failed'
            "#,
            params![turn_id, now],
        )
        .map_err(|error| error.to_string())?;
        conn.execute(
            r#"
            UPDATE strands
            SET last_seen_strand_seq = CASE
                  WHEN last_seen_strand_seq > ?2 THEN last_seen_strand_seq
                  ELSE ?2
                END,
                updated_at = ?3
            WHERE id = (SELECT strand_id FROM turns WHERE id = ?1)
            "#,
            params![turn_id, last_seen_strand_seq, now],
        )
        .map_err(|error| error.to_string())?;
        Database::new(&conn)
            .turn_by_id(turn_id)?
            .ok_or_else(|| "failed turn missing".to_string())
    }
}

pub(super) fn provider_incident_key(strand_id: &str) -> String {
    format!("{}:strand:{strand_id}", catalog::PROVIDER_TURN_FAILED.code)
}

pub(super) fn runtime_incident_key(strand_id: &str) -> String {
    format!("{}:strand:{strand_id}", catalog::RUNTIME_TURN_FAILED.code)
}

pub(super) fn open_runtime_incident(
    conn: &rusqlite::Connection,
    strand_id: &str,
    turn_id: &str,
    failure: RuntimeFault<'_>,
) -> Result<SantiError, String> {
    Database::new(conn).open_incident(IncidentDraft {
        incident_key: runtime_incident_key(strand_id),
        descriptor: catalog::RUNTIME_TURN_FAILED,
        scope: ErrorScope::new("strand", strand_id),
        source: ErrorSource::new("santi-core", failure.operation),
        message: "turn failed inside the runtime".to_string(),
        context: json!({
            "schema": "santi.error.runtime_turn.v1",
            "turn_id": turn_id,
            "operation": failure.operation,
            "detail": bounded_provider_detail(failure.detail),
            "trace": format!("log://turn/{turn_id}"),
        }),
    })
}

pub(super) fn bounded_provider_detail(detail: &str) -> String {
    if detail.len() <= PROVIDER_DETAIL_BYTES {
        return detail.to_string();
    }
    let suffix = " [truncated]";
    let mut end = PROVIDER_DETAIL_BYTES.saturating_sub(suffix.len());
    while end > 0 && !detail.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &detail[..end], suffix)
}
