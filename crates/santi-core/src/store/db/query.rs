use crate::store::rows::*;
use crate::{Compact, StrandEffect, ThinkingSpan, ToolCall, ToolResult, Turn};
use rusqlite::{OptionalExtension, params};

use super::*;

impl<'a> Database<'a> {
    pub(in crate::store) fn turn_by_id(&self, turn_id: &str) -> Result<Option<Turn>, String> {
        self.conn
            .query_row(
                r#"
        SELECT id, strand_id, trigger_type, trigger_ref,
               base_strand_seq, end_strand_seq, status, error_text,
               created_at, updated_at, finished_at
        FROM turns
        WHERE id = ?1
        LIMIT 1
        "#,
                params![turn_id],
                Turn::decode,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub(in crate::store) fn compact_by_id(
        &self,
        compact_id: &str,
    ) -> Result<Option<Compact>, String> {
        self.conn
            .query_row(
                r#"
        SELECT id, strand_id, summary, start_message_id, end_message_id, created_at, metadata
        FROM compacts WHERE id = ?1 LIMIT 1
        "#,
                params![compact_id],
                Compact::decode,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub(in crate::store) fn turn_strand_id(&self, turn_id: &str) -> Result<String, String> {
        self.conn
            .query_row(
                "SELECT strand_id FROM turns WHERE id = ?1 LIMIT 1",
                params![turn_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())
    }

    pub(in crate::store) fn call_soul_id(&self, tool_call_id: &str) -> Result<String, String> {
        self.conn
            .query_row(
                r#"
        SELECT t.strand_id
        FROM tool_calls c
        JOIN turns t ON t.id = c.turn_id
        WHERE c.id = ?1
        LIMIT 1
        "#,
                params![tool_call_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())
    }

    pub(in crate::store) fn tool_call_by_id(
        &self,
        tool_call_id: &str,
    ) -> Result<Option<ToolCall>, String> {
        self.conn.query_row(
        "SELECT id, turn_id, tool_name, arguments, created_at FROM tool_calls WHERE id = ?1 LIMIT 1",
        params![tool_call_id],
        ToolCall::decode,
    )
    .optional()
    .map_err(|error| error.to_string())
    }

    pub(in crate::store) fn regenerable_replay_material(
        &self,
        tool_call_id: &str,
    ) -> Result<(Option<serde_json::Value>, Option<String>), String> {
        self.conn
            .query_row(
                "SELECT blob, item_id FROM provider_replay_material \
         WHERE tool_call_id = ?1 AND kind = 'regenerable' LIMIT 1",
                params![tool_call_id],
                |row| {
                    let blob: Option<String> = row.get(0)?;
                    let item_id: Option<String> = row.get(1)?;
                    Ok((blob, item_id))
                },
            )
            .optional()
            .map_err(|error| error.to_string())
            .map(|found| match found {
                Some((blob, item_id)) => {
                    (blob.and_then(|b| serde_json::from_str(&b).ok()), item_id)
                }
                None => (None, None),
            })
    }

    pub(in crate::store) fn tool_result_by_id(
        &self,
        tool_result_id: &str,
    ) -> Result<Option<ToolResult>, String> {
        self.conn.query_row(
        "SELECT id, tool_call_id, output, error_text, created_at FROM tool_results WHERE id = ?1 LIMIT 1",
        params![tool_result_id],
        ToolResult::decode,
    )
    .optional()
    .map_err(|error| error.to_string())
    }

    pub(in crate::store) fn thinking_span_by_id(
        &self,
        thinking_span_id: &str,
    ) -> Result<Option<ThinkingSpan>, String> {
        self.conn
            .query_row(
                r#"
        SELECT id, turn_id, provider_response_id, state, summary, completion_reason,
               error_text, created_at, updated_at, finished_at
        FROM thinking_spans
        WHERE id = ?1
        LIMIT 1
        "#,
                params![thinking_span_id],
                ThinkingSpan::decode,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub(in crate::store) fn message_seq_in_strand(
        &self,
        strand_id: &str,
        message_id: &str,
    ) -> Result<Option<i64>, String> {
        self.conn
            .query_row(
                r#"
        SELECT strand_seq FROM r_strand_entries
        WHERE strand_id = ?1 AND target_type = 'message' AND target_id = ?2
        LIMIT 1
        "#,
                params![strand_id, message_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub(in crate::store) fn compacts_for_strand(
        &self,
        strand_id: &str,
    ) -> Result<Vec<Compact>, String> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
            SELECT id, strand_id, summary, start_message_id, end_message_id, created_at, metadata
            FROM compacts
            WHERE strand_id = ?1
            "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![strand_id], Compact::decode)
            .map_err(|error| error.to_string())?;
        collect_rows(rows)
    }

    pub(in crate::store) fn strand_effects(
        &self,
        strand_id: &str,
    ) -> Result<Vec<StrandEffect>, String> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
            SELECT id, strand_id, turn_id, tool_call_id, effect_type, state,
                   result_ref, error_text, created_at, updated_at, dispatched_at, settled_at
            FROM strand_effects
            WHERE strand_id = ?1
            ORDER BY created_at ASC
            "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![strand_id], StrandEffect::decode)
            .map_err(|error| error.to_string())?;
        collect_rows(rows)
    }
}
