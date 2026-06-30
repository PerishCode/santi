use rusqlite::params;
use serde_json::{Value, json};

use super::{
    SantiStore,
    db::{
        append_entry_in_tx, call_soul_id, thinking_span_by_id, thinking_spans_for_turn,
        tool_call_by_id, tool_calls_for_turn, tool_result_by_id, tool_results_for_turn, turn_by_id,
        turn_soul_session_id,
    },
};
use crate::{
    SoulSessionEntry, SoulSessionTargetType, ThinkingCompletionReason, ThinkingSpan,
    ThinkingSpanState, ToolCall, ToolResult, Turn, prefixed_id, timestamp_now,
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
        let soul_session_id = turn_soul_session_id(&tx, turn_id)?;
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
        append_entry_in_tx(
            &tx,
            &soul_session_id,
            SoulSessionTargetType::Thinking,
            &thinking_id,
        )?;
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
    ) -> Result<ToolCall, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let now = timestamp_now();
        let soul_session_id = turn_soul_session_id(&tx, turn_id)?;
        tx.execute(
            r#"
            INSERT INTO tool_calls (id, turn_id, tool_name, arguments, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![
                tool_call_id,
                turn_id,
                tool_name,
                serde_json::to_string(arguments).map_err(|error| error.to_string())?,
                now
            ],
        )
        .map_err(|error| error.to_string())?;
        append_entry_in_tx(
            &tx,
            &soul_session_id,
            SoulSessionTargetType::ToolCall,
            tool_call_id,
        )?;
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
        let soul_session_id = call_soul_id(&tx, tool_call_id)?;
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
            &soul_session_id,
            SoulSessionTargetType::ToolResult,
            &tool_result_id,
        )?;
        tx.commit().map_err(|error| error.to_string())?;
        tool_result_by_id(&conn, &tool_result_id)?
            .ok_or_else(|| "created tool_result missing".to_string())
    }

    pub fn complete_turn(
        &self,
        turn_id: &str,
        assistant_message_seq: i64,
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
                end_soul_session_seq = (
                  SELECT next_seq - 1 FROM soul_sessions WHERE id = turns.soul_session_id
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
            UPDATE soul_sessions
            SET last_seen_session_seq = ?2,
                provider_state = ?3,
                updated_at = ?4
            WHERE id = (SELECT soul_session_id FROM turns WHERE id = ?1)
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
            SET end_soul_session_seq = (
                  SELECT next_seq - 1 FROM soul_sessions WHERE id = turns.soul_session_id
                ),
                updated_at = ?2
            WHERE id = ?1 AND status = 'failed'
            "#,
            params![turn_id, now],
        )
        .map_err(|error| error.to_string())?;
        conn.execute(
            r#"
            UPDATE soul_sessions
            SET last_seen_session_seq = CASE
                  WHEN last_seen_session_seq > ?2 THEN last_seen_session_seq
                  ELSE ?2
                END,
                updated_at = ?3
            WHERE id = (SELECT soul_session_id FROM turns WHERE id = ?1)
            "#,
            params![turn_id, last_seen_session_seq, now],
        )
        .map_err(|error| error.to_string())?;
        turn_by_id(&conn, turn_id)?.ok_or_else(|| "failed turn missing".to_string())
    }

    /// The soul-session seq the turn was opened at (its in-flight user message).
    /// Tool entries above this belong to the still-running turn.
    pub fn turn_base_soul_session_seq(&self, turn_id: &str) -> Result<i64, String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT base_soul_session_seq FROM turns WHERE id = ?1 LIMIT 1",
            params![turn_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
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

    pub(super) fn append_soul_session_entry(
        &self,
        soul_session_id: &str,
        target_type: SoulSessionTargetType,
        target_id: &str,
    ) -> Result<SoulSessionEntry, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let entry = append_entry_in_tx(&tx, soul_session_id, target_type, target_id)?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(entry)
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
