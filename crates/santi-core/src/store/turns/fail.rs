use crate::store::{Misfire, Store, Stumble, db::Database};
use crate::{Fault, catalog, now, turn::Turn};
use rusqlite::params;
use serde_json::json;

use super::*;
use crate::effect;

impl Store {
    pub(crate) fn fail_provider_turn(
        &self,
        turn: &str,
        error: &str,
        failure: Misfire<'_>,
    ) -> Result<(Turn, Fault), String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let strand: String = tx
            .query_row(
                "SELECT strand_id FROM turns WHERE id = ?1",
                params![turn],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let now = now();
        tx.execute(
            r#"
            UPDATE turns
            SET status = 'failed', error_text = ?2, updated_at = ?3, finished_at = ?3
            WHERE id = ?1
            "#,
            params![turn, error, now],
        )
        .map_err(|error| error.to_string())?;
        let error = Database::new(&tx).open(santi_error::Draft {
            key: provider_incident_key(&strand),
            descriptor: catalog::PROVIDER_TURN_FAILED,
            scope: santi_error::Scope::new("strand", &strand),
            source: santi_error::Source::new("santi-provider", failure.operation),
            message: format!("provider {} failed", failure.stage),
            context: json!({
                "turn": turn,
                "provider": failure.provider,
                "model": failure.model,
                "stage": failure.stage,
                "round": failure.round,
                "detail": clipped(failure.detail),
                "trace": format!("log://turn/{turn}"),
            }),
        })?;
        Database::new(&tx).reconcile(
            turn,
            effect::Reason::TurnFailedBeforeDispatch,
            effect::Reason::TurnFailedDuringDispatch,
            &now,
        )?;
        Database::new(&tx).fail(turn, error.incident.as_deref(), &now)?;
        tx.commit().map_err(|error| error.to_string())?;
        let turn = Database::new(&conn)
            .turn(turn)?
            .ok_or_else(|| "failed turn missing".to_string())?;
        Ok((turn, error))
    }

    pub(crate) fn fail_runtime_turn(
        &self,
        turn: &str,
        error: &str,
        failure: Stumble<'_>,
    ) -> Result<(Turn, Fault), String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let strand: String = tx
            .query_row(
                "SELECT strand_id FROM turns WHERE id = ?1",
                params![turn],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let now = now();
        tx.execute(
            r#"
            UPDATE turns
            SET status = 'failed', error_text = ?2, updated_at = ?3, finished_at = ?3
            WHERE id = ?1
            "#,
            params![turn, error, now],
        )
        .map_err(|error| error.to_string())?;
        let error = open_runtime_incident(&tx, &strand, turn, failure)?;
        Database::new(&tx).reconcile(
            turn,
            effect::Reason::TurnFailedBeforeDispatch,
            effect::Reason::TurnFailedDuringDispatch,
            &now,
        )?;
        Database::new(&tx).fail(turn, error.incident.as_deref(), &now)?;
        tx.commit().map_err(|error| error.to_string())?;
        let turn = Database::new(&conn)
            .turn(turn)?
            .ok_or_else(|| "failed turn missing".to_string())?;
        Ok((turn, error))
    }

    pub fn fail(&self, turn: &str, error: &str) -> Result<Turn, String> {
        self.fail_turn_with_incident(turn, error, None)
    }

    pub(crate) fn fail_turn_with_incident(
        &self,
        turn: &str,
        error: &str,
        incident: Option<&str>,
    ) -> Result<Turn, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let now = now();
        tx.execute(
            r#"
            UPDATE turns
            SET status = 'failed', error_text = ?2, updated_at = ?3, finished_at = ?3
            WHERE id = ?1
            "#,
            params![turn, error, now],
        )
        .map_err(|error| error.to_string())?;
        Database::new(&tx).reconcile(
            turn,
            effect::Reason::TurnFailedBeforeDispatch,
            effect::Reason::TurnFailedDuringDispatch,
            &now,
        )?;
        Database::new(&tx).fail(turn, incident, &now)?;
        tx.commit().map_err(|error| error.to_string())?;
        Database::new(&conn)
            .turn(turn)?
            .ok_or_else(|| "failed turn missing".to_string())
    }

    pub fn finish_failed_turn_context(&self, turn: &str, seen: i64) -> Result<Turn, String> {
        let conn = self.conn.lock().unwrap();
        let now = now();
        conn.execute(
            r#"
            UPDATE turns
            SET end_strand_seq = (
                  SELECT next_seq - 1 FROM strands WHERE id = turns.strand_id
                ),
                updated_at = ?2
            WHERE id = ?1 AND status = 'failed'
            "#,
            params![turn, now],
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
            params![turn, seen, now],
        )
        .map_err(|error| error.to_string())?;
        Database::new(&conn)
            .turn(turn)?
            .ok_or_else(|| "failed turn missing".to_string())
    }
}

pub(super) fn provider_incident_key(strand: &str) -> String {
    format!("{}:strand:{strand}", catalog::PROVIDER_TURN_FAILED.code)
}

pub(super) fn runtime_incident_key(strand: &str) -> String {
    format!("{}:strand:{strand}", catalog::RUNTIME_TURN_FAILED.code)
}

pub(super) fn open_runtime_incident(
    conn: &rusqlite::Connection,
    strand: &str,
    turn: &str,
    failure: Stumble<'_>,
) -> Result<Fault, String> {
    Database::new(conn).open(santi_error::Draft {
        key: runtime_incident_key(strand),
        descriptor: catalog::RUNTIME_TURN_FAILED,
        scope: santi_error::Scope::new("strand", strand),
        source: santi_error::Source::new("santi-core", failure.operation),
        message: "turn failed inside the runtime".to_string(),
        context: json!({
            "schema": "santi.error.runtime_turn.v1",
            "turn": turn,
            "operation": failure.operation,
            "detail": clipped(failure.detail),
            "trace": format!("log://turn/{turn}"),
        }),
    })
}

pub(super) fn clipped(detail: &str) -> String {
    if detail.len() <= BREADTH {
        return detail.to_string();
    }
    let suffix = " [truncated]";
    let mut end = BREADTH.saturating_sub(suffix.len());
    while end > 0 && !detail.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &detail[..end], suffix)
}
