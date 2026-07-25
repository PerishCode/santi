use crate::Ruled;
use crate::store::{Misfire, Store, Stumble, db::Database};
use crate::{Fault, now, turn::Turn};
use rusqlite::params;
use serde_json::json;

use super::*;

impl Store {
    pub(crate) fn misfire(
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
            key: crate::turn::Error::Provider
                .descriptor()
                .key("strand", &strand),
            descriptor: crate::turn::Error::Provider.descriptor(),
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
            "turn_failed_before_dispatch",
            "turn_failed_during_dispatch",
            &now,
        )?;
        Database::new(&tx).fail(turn, error.incident.as_deref(), &now)?;
        tx.commit().map_err(|error| error.to_string())?;
        let turn = Database::new(&conn)
            .turn(turn)?
            .ok_or_else(|| "failed turn missing".to_string())?;
        Ok((turn, error))
    }

    pub(crate) fn stumble(
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
        let error = indict(&tx, &strand, turn, failure)?;
        Database::new(&tx).reconcile(
            turn,
            "turn_failed_before_dispatch",
            "turn_failed_during_dispatch",
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
        self.condemn(turn, error, None)
    }

    pub(crate) fn condemn(
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
            "turn_failed_before_dispatch",
            "turn_failed_during_dispatch",
            &now,
        )?;
        Database::new(&tx).fail(turn, incident, &now)?;
        tx.commit().map_err(|error| error.to_string())?;
        Database::new(&conn)
            .turn(turn)?
            .ok_or_else(|| "failed turn missing".to_string())
    }

    pub fn seal(&self, turn: &str, seen: i64) -> Result<Turn, String> {
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

pub(super) fn indict(
    conn: &rusqlite::Connection,
    strand: &str,
    turn: &str,
    failure: Stumble<'_>,
) -> Result<Fault, String> {
    Database::new(conn).open(santi_error::Draft {
        key: crate::turn::Error::Runtime
            .descriptor()
            .key("strand", strand),
        descriptor: crate::turn::Error::Runtime.descriptor(),
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
