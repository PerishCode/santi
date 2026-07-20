use rusqlite::params;

use super::{
    RuntimeFault, SantiStore, StartedTurn,
    db::{Database, drain_inbox_in_tx},
};
use crate::{
    EffectTransitionReason, ThinkingSpan, ToolCall, ToolResult, Turn, prefixed_id, timestamp_now,
};

mod completion;

pub use completion::Completion;

const PROVIDER_DETAIL_BYTES: usize = 4096;
mod fail;
use fail::{open_runtime_incident, provider_incident_key, runtime_incident_key};

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
