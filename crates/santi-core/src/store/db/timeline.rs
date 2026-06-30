use rusqlite::{Connection, params};

use crate::{ThinkingSpan, ToolCall, ToolResult, Turn};

use super::{
    collect_rows, map_thinking_span_row, map_tool_call_row, map_tool_result_row, map_turn_row,
};

pub(in crate::store) fn turns_for_soul_session(
    conn: &Connection,
    soul_session_id: &str,
) -> Result<Vec<Turn>, String> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, soul_session_id, trigger_type, trigger_ref, input_through_session_seq,
                   base_soul_session_seq, end_soul_session_seq, status, error_text,
                   created_at, updated_at, finished_at
            FROM turns
            WHERE soul_session_id = ?1
            ORDER BY created_at ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![soul_session_id], map_turn_row)
        .map_err(|error| error.to_string())?;
    collect_rows(rows)
}

pub(in crate::store) fn soul_tool_calls(
    conn: &Connection,
    soul_session_id: &str,
) -> Result<Vec<ToolCall>, String> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT c.id, c.turn_id, c.tool_name, c.arguments, c.provider_item, c.item_id, c.response_id, c.created_at
            FROM tool_calls c
            JOIN turns t ON t.id = c.turn_id
            WHERE t.soul_session_id = ?1
            ORDER BY c.created_at ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![soul_session_id], map_tool_call_row)
        .map_err(|error| error.to_string())?;
    collect_rows(rows)
}

pub(in crate::store) fn tool_calls_for_turn(
    conn: &Connection,
    turn_id: &str,
) -> Result<Vec<ToolCall>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, turn_id, tool_name, arguments, provider_item, item_id, response_id, created_at FROM tool_calls WHERE turn_id = ?1 ORDER BY created_at ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![turn_id], map_tool_call_row)
        .map_err(|error| error.to_string())?;
    collect_rows(rows)
}

pub(in crate::store) fn soul_thinking_spans(
    conn: &Connection,
    soul_session_id: &str,
) -> Result<Vec<ThinkingSpan>, String> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT s.id, s.turn_id, s.provider_response_id, s.state, s.summary,
                   s.completion_reason, s.error_text, s.created_at, s.updated_at,
                   s.finished_at
            FROM thinking_spans s
            JOIN turns t ON t.id = s.turn_id
            WHERE t.soul_session_id = ?1
            ORDER BY s.created_at ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![soul_session_id], map_thinking_span_row)
        .map_err(|error| error.to_string())?;
    collect_rows(rows)
}

pub(in crate::store) fn thinking_spans_for_turn(
    conn: &Connection,
    turn_id: &str,
) -> Result<Vec<ThinkingSpan>, String> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, turn_id, provider_response_id, state, summary, completion_reason,
                   error_text, created_at, updated_at, finished_at
            FROM thinking_spans
            WHERE turn_id = ?1
            ORDER BY created_at ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![turn_id], map_thinking_span_row)
        .map_err(|error| error.to_string())?;
    collect_rows(rows)
}

pub(in crate::store) fn soul_tool_results(
    conn: &Connection,
    soul_session_id: &str,
) -> Result<Vec<ToolResult>, String> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT r.id, r.tool_call_id, r.output, r.error_text, r.created_at
            FROM tool_results r
            JOIN tool_calls c ON c.id = r.tool_call_id
            JOIN turns t ON t.id = c.turn_id
            WHERE t.soul_session_id = ?1
            ORDER BY r.created_at ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![soul_session_id], map_tool_result_row)
        .map_err(|error| error.to_string())?;
    collect_rows(rows)
}

pub(in crate::store) fn tool_results_for_turn(
    conn: &Connection,
    turn_id: &str,
) -> Result<Vec<ToolResult>, String> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT r.id, r.tool_call_id, r.output, r.error_text, r.created_at
            FROM tool_results r
            JOIN tool_calls c ON c.id = r.tool_call_id
            WHERE c.turn_id = ?1
            ORDER BY r.created_at ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![turn_id], map_tool_result_row)
        .map_err(|error| error.to_string())?;
    collect_rows(rows)
}
