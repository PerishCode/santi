use rusqlite::params;
use serde_json::Value;

use super::{
    SantiStore,
    db::{
        append_entry_in_tx, call_soul_id, message_by_id, thinking_span_by_id, tool_call_by_id,
        tool_result_by_id, turn_strand_id,
    },
};
use crate::{
    StrandTargetType, ThinkingCompletionReason, ThinkingSpan, ThinkingSpanState, ToolCall,
    ToolResult, prefixed_id, timestamp_now,
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
        // Neutral occurrence: no provider plumbing (PHASE-09 decision #9).
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
        // Adaptor-owned replay material, side-stored. A function_call's raw item
        // is REGENERABLE (the adaptor can synthesize it from the neutral fields),
        // so a bad blob is a droppable cache-miss, never a durable poison.
        let provider_item_text = provenance
            .item
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        if provider_item_text.is_some()
            || provenance.item_id.is_some()
            || provenance.response_id.is_some()
        {
            tx.execute(
                r#"
                INSERT INTO provider_replay_material
                    (tool_call_id, provider_family, kind, blob, item_id, response_id, schema_version, created_at)
                VALUES (?1, ?2, 'regenerable', ?3, ?4, ?5, ?6, ?7)
                "#,
                params![
                    tool_call_id,
                    provenance.provider_family,
                    provider_item_text,
                    provenance.item_id,
                    provenance.response_id,
                    crate::SCHEMA_VERSION,
                    now
                ],
            )
            .map_err(|error| error.to_string())?;
        }
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
    ) -> Result<crate::StrandMessage, String> {
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
