use rusqlite::params;
use serde_json::json;

use super::{
    ProviderFault, RuntimeFault, SantiStore, StartedTurn,
    db::{Database, drain_inbox_in_tx},
};
use crate::{
    EffectTransitionReason, ErrorScope, ErrorSource, IncidentDraft, SantiError, ThinkingSpan,
    ToolCall, ToolResult, Turn, catalog, prefixed_id, timestamp_now,
};

mod completion;

pub use completion::Completion;

const PROVIDER_DETAIL_BYTES: usize = 4096;

impl SantiStore {
    pub fn try_start_turn(
        &self,
        strand_id: &str,
        trigger_type: &str,
        trigger_ref: Option<&str>,
    ) -> Result<Option<StartedTurn>, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let running: Option<i64> = tx
            .query_row(
                "SELECT 1 FROM turns WHERE strand_id = ?1 AND status = 'running' LIMIT 1",
                params![strand_id],
                |row| row.get(0),
            )
            .ok();
        if running.is_some() {
            return Ok(None);
        }
        let turn_id = prefixed_id("turn");
        let drained = drain_inbox_in_tx(&tx, strand_id, &turn_id)?;
        if drained.messages.is_empty() {
            return Ok(None);
        }
        let now = timestamp_now();
        tx.execute(
            r#"
            INSERT INTO turns (
              id, strand_id, trigger_type, trigger_ref,
              base_strand_seq, end_strand_seq, status, error_text,
              created_at, updated_at, finished_at
            )
            SELECT ?1, id, ?3, ?4, next_seq - 1, NULL, 'running', NULL, ?5, ?5, NULL
            FROM strands WHERE id = ?2
            "#,
            params![turn_id, strand_id, trigger_type, trigger_ref, now],
        )
        .map_err(|error| error.to_string())?;
        Database::new(&tx).begin_turn(strand_id, &turn_id, &drained.inbox_ids, None)?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(Some(StartedTurn {
            turn: Database::new(&conn)
                .turn_by_id(&turn_id)?
                .ok_or_else(|| "created turn missing".to_string())?,
            drained_messages: drained.messages,
        }))
    }

    pub fn latest_turn(&self, strand_id: &str) -> Result<Option<Turn>, String> {
        let conn = self.conn.lock().unwrap();
        let id: Option<String> = conn
            .query_row(
                "SELECT id FROM turns WHERE strand_id = ?1 ORDER BY created_at DESC, id DESC LIMIT 1",
                params![strand_id],
                |row| row.get(0),
            )
            .ok();
        match id {
            Some(id) => Database::new(&conn).turn_by_id(&id),
            None => Ok(None),
        }
    }

    pub fn reconcile_orphaned_turns(&self) -> Result<usize, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let now = timestamp_now();
        let mut stmt = tx
            .prepare("SELECT id, strand_id FROM turns WHERE status = 'running'")
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        drop(stmt);
        for (turn_id, strand_id) in &rows {
            Database::new(&tx).reconcile_effects(
                turn_id,
                EffectTransitionReason::RestartBeforeDispatch,
                EffectTransitionReason::RestartDuringDispatch,
                &now,
            )?;
            tx.execute(
                r#"
                UPDATE turns
                SET status = 'failed', error_text = 'interrupted by restart',
                    updated_at = ?2, finished_at = ?2
                WHERE id = ?1 AND status = 'running'
                "#,
                params![turn_id, now],
            )
            .map_err(|error| error.to_string())?;
            let error = open_runtime_incident(
                &tx,
                strand_id,
                turn_id,
                RuntimeFault {
                    operation: "turn.restart_reconcile",
                    detail: "interrupted by restart",
                },
            )?;
            Database::new(&tx).fail_turn(turn_id, error.incident_id.as_deref(), &now)?;
        }
        tx.commit().map_err(|error| error.to_string())?;
        Ok(rows.len())
    }

    pub fn running_turn_count(&self) -> Result<usize, String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM turns WHERE status = 'running'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count as usize)
        .map_err(|error| error.to_string())
    }

    pub fn strands_with_pending_requests(&self) -> Result<Vec<String>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT DISTINCT strand_id FROM strand_inbox")
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map([], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|error| error.to_string())?);
        }
        Ok(out)
    }

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

    pub fn tool_calls_for_turn(&self, turn_id: &str) -> Result<Vec<ToolCall>, String> {
        let conn = self.conn.lock().unwrap();
        Database::new(&conn).tool_calls_for_turn(turn_id)
    }

    pub fn thinking_spans_for_turn(&self, turn_id: &str) -> Result<Vec<ThinkingSpan>, String> {
        let conn = self.conn.lock().unwrap();
        Database::new(&conn).thinking_spans_for_turn(turn_id)
    }

    pub fn tool_results_for_turn(&self, turn_id: &str) -> Result<Vec<ToolResult>, String> {
        let conn = self.conn.lock().unwrap();
        Database::new(&conn).tool_results_for_turn(turn_id)
    }
}

fn provider_incident_key(strand_id: &str) -> String {
    format!("{}:strand:{strand_id}", catalog::PROVIDER_TURN_FAILED.code)
}

fn runtime_incident_key(strand_id: &str) -> String {
    format!("{}:strand:{strand_id}", catalog::RUNTIME_TURN_FAILED.code)
}

fn open_runtime_incident(
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

fn bounded_provider_detail(detail: &str) -> String {
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
