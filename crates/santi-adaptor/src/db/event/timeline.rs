use rusqlite::params;

use santi_model::turn::Turn;

use super::{Database, Decode, collected};
use santi_model::{thinking, tool};

impl Database<'_> {
    pub fn turns(&self, strand: &str) -> Result<Vec<Turn>, String> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
            SELECT id, strand_id, trigger_type, trigger_ref,
                   base_strand_seq, end_strand_seq, status, error_text,
                   created_at, updated_at, finished_at
            FROM turns
            WHERE strand_id = ?1
            ORDER BY created_at ASC
            "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![strand], Turn::decode)
            .map_err(|error| error.to_string())?;
        collected(rows)
    }

    pub fn calls(&self, strand: &str) -> Result<Vec<tool::Call>, String> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
            SELECT c.id, c.turn_id, c.tool_name, c.arguments, c.created_at
            FROM r_strand_entries e
            JOIN tool_calls c ON c.id = e.target_id
            WHERE e.strand_id = ?1 AND e.target_type = 'tool_call'
            ORDER BY e.strand_seq ASC
            "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![strand], tool::Call::decode)
            .map_err(|error| error.to_string())?;
        collected(rows)
    }

    pub fn called(&self, turn: &str) -> Result<Vec<tool::Call>, String> {
        let mut stmt = self
            .conn
        .prepare(
            "SELECT id, turn_id, tool_name, arguments, created_at FROM tool_calls WHERE turn_id = ?1 ORDER BY created_at ASC",
        )
        .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![turn], tool::Call::decode)
            .map_err(|error| error.to_string())?;
        collected(rows)
    }

    pub fn thinking(&self, strand: &str) -> Result<Vec<thinking::Span>, String> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
            SELECT s.id, s.turn_id, s.provider_response_id, s.state, s.summary,
                   s.completion_reason, s.error_text, s.created_at, s.updated_at,
                   s.finished_at
            FROM r_strand_entries e
            JOIN thinking_spans s ON s.id = e.target_id
            WHERE e.strand_id = ?1 AND e.target_type = 'thinking'
            ORDER BY e.strand_seq ASC
            "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![strand], thinking::Span::decode)
            .map_err(|error| error.to_string())?;
        collected(rows)
    }

    pub fn thought(&self, turn: &str) -> Result<Vec<thinking::Span>, String> {
        let mut stmt = self
            .conn
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
            .query_map(params![turn], thinking::Span::decode)
            .map_err(|error| error.to_string())?;
        collected(rows)
    }

    pub fn results(&self, strand: &str) -> Result<Vec<tool::Reply>, String> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
            SELECT r.id, r.tool_call_id, r.output, r.error_text, r.created_at
            FROM r_strand_entries e
            JOIN tool_results r ON r.id = e.target_id
            WHERE e.strand_id = ?1 AND e.target_type = 'tool_result'
            ORDER BY e.strand_seq ASC
            "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![strand], tool::Reply::decode)
            .map_err(|error| error.to_string())?;
        collected(rows)
    }

    pub fn replied(&self, turn: &str) -> Result<Vec<tool::Reply>, String> {
        let mut stmt = self
            .conn
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
            .query_map(params![turn], tool::Reply::decode)
            .map_err(|error| error.to_string())?;
        collected(rows)
    }
}
