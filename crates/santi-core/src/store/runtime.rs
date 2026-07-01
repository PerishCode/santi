use rusqlite::params;
use serde_json::{Value, json};

use super::{
    SantiStore, StartedTurn,
    db::{
        append_entry_in_tx, call_soul_id, drain_inbox_in_tx, message_by_id, thinking_span_by_id,
        thinking_spans_for_turn, tool_call_by_id, tool_calls_for_turn, tool_result_by_id,
        tool_results_for_turn, turn_by_id, turn_strand_id,
    },
};
use crate::{
    StrandTargetType, ThinkingCompletionReason, ThinkingSpan, ThinkingSpanState, ToolCall,
    ToolResult, Turn, prefixed_id, timestamp_now,
};

impl SantiStore {
    pub fn append_thinking_span(
        &self,
        turn_id: &str,
        provider_response_id: Option<String>,
    ) -> Result<ThinkingSpan, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let thinking_id = prefixed_id("thinking");
        let now = timestamp_now();
        let strand_id = turn_strand_id(&tx, turn_id)?;
        tx.execute(
            r#"
            INSERT INTO thinking_spans (
              id, turn_id, provider_response_id, state, summary, completion_reason,
              error_text, created_at, updated_at, finished_at
            )
            VALUES (?1, ?2, ?3, 'running', NULL, NULL, NULL, ?4, ?4, NULL)
            "#,
            params![thinking_id, turn_id, provider_response_id, now],
        )
        .map_err(|error| error.to_string())?;
        append_entry_in_tx(&tx, &strand_id, StrandTargetType::Thinking, &thinking_id)?;
        tx.commit().map_err(|error| error.to_string())?;
        thinking_span_by_id(&conn, &thinking_id)?
            .ok_or_else(|| "created thinking_span missing".to_string())
    }

    pub fn update_thinking_span_response(
        &self,
        thinking_span_id: &str,
        provider_response_id: Option<String>,
    ) -> Result<Option<ThinkingSpan>, String> {
        let conn = self.conn.lock().unwrap();
        let now = timestamp_now();
        conn.execute(
            r#"
            UPDATE thinking_spans
            SET provider_response_id = COALESCE(?2, provider_response_id),
                updated_at = ?3
            WHERE id = ?1 AND state = 'running'
            "#,
            params![thinking_span_id, provider_response_id, now],
        )
        .map_err(|error| error.to_string())?;
        thinking_span_by_id(&conn, thinking_span_id)
    }

    pub fn update_thinking_span_summary(
        &self,
        thinking_span_id: &str,
        summary: String,
    ) -> Result<Option<ThinkingSpan>, String> {
        let conn = self.conn.lock().unwrap();
        let now = timestamp_now();
        conn.execute(
            r#"
            UPDATE thinking_spans
            SET summary = ?2,
                updated_at = ?3
            WHERE id = ?1 AND state <> 'failed'
            "#,
            params![thinking_span_id, summary, now],
        )
        .map_err(|error| error.to_string())?;
        thinking_span_by_id(&conn, thinking_span_id)
    }

    pub fn complete_thinking_span(
        &self,
        thinking_span_id: &str,
        completion_reason: ThinkingCompletionReason,
    ) -> Result<Option<ThinkingSpan>, String> {
        self.finish_thinking_span(
            thinking_span_id,
            ThinkingSpanState::Completed,
            Some(completion_reason),
            None,
        )
    }

    pub fn fail_thinking_span(
        &self,
        thinking_span_id: &str,
        error_text: String,
    ) -> Result<Option<ThinkingSpan>, String> {
        self.finish_thinking_span(
            thinking_span_id,
            ThinkingSpanState::Failed,
            None,
            Some(error_text),
        )
    }

    pub fn append_tool_call(
        &self,
        turn_id: &str,
        tool_call_id: &str,
        tool_name: &str,
        arguments: &Value,
        provenance: &crate::ToolCallProvenance,
    ) -> Result<ToolCall, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let now = timestamp_now();
        let strand_id = turn_strand_id(&tx, turn_id)?;
        let provider_item_text = provenance
            .item
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        tx.execute(
            r#"
            INSERT INTO tool_calls (id, turn_id, tool_name, arguments, provider_item, item_id, response_id, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                tool_call_id,
                turn_id,
                tool_name,
                serde_json::to_string(arguments).map_err(|error| error.to_string())?,
                provider_item_text,
                provenance.item_id,
                provenance.response_id,
                now
            ],
        )
        .map_err(|error| error.to_string())?;
        append_entry_in_tx(&tx, &strand_id, StrandTargetType::ToolCall, tool_call_id)?;
        tx.commit().map_err(|error| error.to_string())?;
        tool_call_by_id(&conn, tool_call_id)?.ok_or_else(|| "created tool_call missing".to_string())
    }

    pub fn append_tool_result(
        &self,
        tool_call_id: &str,
        output: Option<Value>,
        error_text: Option<String>,
    ) -> Result<ToolResult, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let tool_result_id = prefixed_id("tool_result");
        let now = timestamp_now();
        let strand_id = call_soul_id(&tx, tool_call_id)?;
        let output_text = output
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        tx.execute(
            r#"
            INSERT INTO tool_results (id, tool_call_id, output, error_text, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![tool_result_id, tool_call_id, output_text, error_text, now],
        )
        .map_err(|error| error.to_string())?;
        append_entry_in_tx(
            &tx,
            &strand_id,
            StrandTargetType::ToolResult,
            &tool_result_id,
        )?;
        tx.commit().map_err(|error| error.to_string())?;
        tool_result_by_id(&conn, &tool_result_id)?
            .ok_or_else(|| "created tool_result missing".to_string())
    }

    /// Append a per-round assistant text segment to the strand's timeline. This
    /// is the soul's speech in this round — the interleaved replay log (DC4b/DC6)
    /// AND the operator-visible conversational projection are the SAME entry now
    /// that both read `r_strand_entries` (no separate lumped end-of-turn record).
    pub fn append_soul_assistant_text(
        &self,
        strand_id: &str,
        text: &str,
    ) -> Result<crate::SessionMessage, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let soul_id: String = tx
            .query_row(
                "SELECT soul_id FROM strands WHERE id = ?1 LIMIT 1",
                params![strand_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let message_id = prefixed_id("msg");
        let now = timestamp_now();
        let content_json = serde_json::to_string(&crate::MessageContent::text(text))
            .map_err(|error| error.to_string())?;
        tx.execute(
            r#"
            INSERT INTO messages (
              id, actor_type, actor_id, message_kind, content, state, version, is_request,
              deleted_at, created_at, updated_at
            )
            VALUES (?1, 'soul', ?2, 'text', ?3, 'fixed', 1, 0, NULL, ?4, ?4)
            "#,
            params![message_id, soul_id, content_json, now],
        )
        .map_err(|error| error.to_string())?;
        append_entry_in_tx(&tx, strand_id, StrandTargetType::Message, &message_id)?;
        tx.commit().map_err(|error| error.to_string())?;
        message_by_id(&conn, &message_id)?.ok_or_else(|| "created message missing".to_string())
    }

    /// Atomically start a turn for a strand IFF (a) no turn is currently
    /// running for it and (b) it is "behind" — its inbox is non-empty. This is
    /// the lynchpin of the drive model: it makes "one present per thread of
    /// experience" an invariant (the store mutex serializes drain+guard+insert),
    /// and lets ingest pokes and completion re-checks race harmlessly. Draining
    /// the inbox INTO the timeline is part of this same atomic step — commit and
    /// turn-start happen together, so a committed-but-uncovered REQUEST can
    /// never exist. Returns the started turn (with what it drained), or None.
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
        let drained_messages = drain_inbox_in_tx(&tx, strand_id)?;
        if drained_messages.is_empty() {
            return Ok(None);
        }
        let turn_id = prefixed_id("turn");
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
        tx.commit().map_err(|error| error.to_string())?;
        Ok(Some(StartedTurn {
            turn: turn_by_id(&conn, &turn_id)?.ok_or_else(|| "created turn missing".to_string())?,
            drained_messages,
        }))
    }

    /// The most recent turn for a strand (the active one under the drive
    /// model — running if busy, else the latest completed). For response shaping.
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
            Some(id) => turn_by_id(&conn, &id),
            None => Ok(None),
        }
    }

    /// On startup, every `running` turn is orphaned (its process is gone).
    /// Reconcile them to an honest "interrupted" terminal — never fabricate a
    /// result — so the strand is idle again and the soul sees the truth.
    pub fn reconcile_orphaned_turns(&self) -> Result<usize, String> {
        let conn = self.conn.lock().unwrap();
        let now = timestamp_now();
        conn.execute(
            r#"
            UPDATE turns
            SET status = 'failed', error_text = 'interrupted by restart',
                updated_at = ?1, finished_at = ?1
            WHERE status = 'running'
            "#,
            params![now],
        )
        .map_err(|error| error.to_string())
    }

    /// Strands that are "behind" (their inbox is non-empty). Used on boot to
    /// re-drive durable requests stranded by a crash (liveness) — the inbox
    /// itself is durable, so this is exactly "which strands still have
    /// something an adaptor enqueued but the driver never drained".
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

    pub fn complete_turn(
        &self,
        turn_id: &str,
        assistant_message_seq: Option<i64>,
        provider: &str,
        provider_response_id: Option<String>,
    ) -> Result<Turn, String> {
        let conn = self.conn.lock().unwrap();
        let now = timestamp_now();
        let provider_state = provider_response_id.map(|response_id| {
            json!({
                "provider": provider,
                "opaque": { "response_id": response_id },
                "schema_version": "santi-v1"
            })
        });
        let provider_state = provider_state
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        conn.execute(
            r#"
            UPDATE turns
            SET status = 'completed',
                end_strand_seq = (
                  SELECT next_seq - 1 FROM strands WHERE id = turns.strand_id
                ),
                updated_at = ?2,
                finished_at = ?2
            WHERE id = ?1
            "#,
            params![turn_id, now],
        )
        .map_err(|error| error.to_string())?;
        conn.execute(
            r#"
            UPDATE strands
            SET last_seen_session_seq = COALESCE(?2, last_seen_session_seq),
                provider_state = ?3,
                updated_at = ?4
            WHERE id = (SELECT strand_id FROM turns WHERE id = ?1)
            "#,
            params![turn_id, assistant_message_seq, provider_state, now],
        )
        .map_err(|error| error.to_string())?;
        turn_by_id(&conn, turn_id)?.ok_or_else(|| "completed turn missing".to_string())
    }

    pub fn fail_turn(&self, turn_id: &str, error_text: &str) -> Result<Turn, String> {
        let conn = self.conn.lock().unwrap();
        let now = timestamp_now();
        conn.execute(
            r#"
            UPDATE turns
            SET status = 'failed', error_text = ?2, updated_at = ?3, finished_at = ?3
            WHERE id = ?1
            "#,
            params![turn_id, error_text, now],
        )
        .map_err(|error| error.to_string())?;
        turn_by_id(&conn, turn_id)?.ok_or_else(|| "failed turn missing".to_string())
    }

    pub fn finish_failed_turn_context(
        &self,
        turn_id: &str,
        last_seen_session_seq: i64,
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
            SET last_seen_session_seq = CASE
                  WHEN last_seen_session_seq > ?2 THEN last_seen_session_seq
                  ELSE ?2
                END,
                updated_at = ?3
            WHERE id = (SELECT strand_id FROM turns WHERE id = ?1)
            "#,
            params![turn_id, last_seen_session_seq, now],
        )
        .map_err(|error| error.to_string())?;
        turn_by_id(&conn, turn_id)?.ok_or_else(|| "failed turn missing".to_string())
    }

    pub fn tool_calls_for_turn(&self, turn_id: &str) -> Result<Vec<ToolCall>, String> {
        let conn = self.conn.lock().unwrap();
        tool_calls_for_turn(&conn, turn_id)
    }

    pub fn thinking_spans_for_turn(&self, turn_id: &str) -> Result<Vec<ThinkingSpan>, String> {
        let conn = self.conn.lock().unwrap();
        thinking_spans_for_turn(&conn, turn_id)
    }

    pub fn tool_results_for_turn(&self, turn_id: &str) -> Result<Vec<ToolResult>, String> {
        let conn = self.conn.lock().unwrap();
        tool_results_for_turn(&conn, turn_id)
    }

    fn finish_thinking_span(
        &self,
        thinking_span_id: &str,
        state: ThinkingSpanState,
        completion_reason: Option<ThinkingCompletionReason>,
        error_text: Option<String>,
    ) -> Result<Option<ThinkingSpan>, String> {
        let conn = self.conn.lock().unwrap();
        let now = timestamp_now();
        conn.execute(
            r#"
            UPDATE thinking_spans
            SET state = ?2,
                completion_reason = ?3,
                error_text = ?4,
                updated_at = ?5,
                finished_at = ?5
            WHERE id = ?1 AND state = 'running'
            "#,
            params![
                thinking_span_id,
                super::rows::thinking_span_state_db(&state),
                completion_reason
                    .as_ref()
                    .map(super::rows::thinking_completion_reason_db),
                error_text,
                now
            ],
        )
        .map_err(|error| error.to_string())?;
        thinking_span_by_id(&conn, thinking_span_id)
    }
}
