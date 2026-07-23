use crate::store::{db::Database, span::Span};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use super::*;
use crate::{message, strand};

pub(super) fn plan_compact_in_tx(
    conn: &Connection,
    strand: &str,
    first: &str,
    last: &str,
) -> Result<Plan, String> {
    for (label, id) in [("from", first), ("to", last)] {
        let message = Database::new(conn)
            .message_record_by_id(id)?
            .ok_or_else(|| format!("compact {label} message not found"))?;
        let is_projected = message.role == message::Role::Soul
            || (message.role == message::Role::System
                && matches!(
                    message.kind,
                    message::Kind::Text | message::Kind::SantiSystem
                ));
        if !is_projected || message.state != message::State::Fixed {
            return Err(format!(
                "compact {label} boundary must be a fixed projected message"
            ));
        }
    }

    let database = Database::new(conn);
    let start_seq = database
        .message_seq_in_strand(strand, first)?
        .ok_or_else(|| "compact from message not in this strand".to_string())?;
    let end_seq = database
        .message_seq_in_strand(strand, last)?
        .ok_or_else(|| "compact to message not in this strand".to_string())?;
    if start_seq > end_seq {
        return Err("compact from must not be after to".to_string());
    }

    let mut absorbed = Vec::new();
    for existing in database.compacts_for_strand(strand)? {
        let (Some(es), Some(ee)) = (
            database.message_seq_in_strand(strand, &existing.first)?,
            database.message_seq_in_strand(strand, &existing.last)?,
        ) else {
            continue;
        };
        if ee < start_seq || es > end_seq {
            continue;
        }
        if start_seq <= es && ee <= end_seq {
            absorbed.push(existing.id);
            continue;
        }
        return Err("compact range partially overlaps an existing compact".to_string());
    }

    let collapsed_count: i64 = conn
        .query_row(
            r#"
            SELECT COUNT(*) FROM r_strand_entries
            WHERE strand_id = ?1 AND strand_seq BETWEEN ?2 AND ?3
            "#,
            params![strand, start_seq, end_seq],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;

    Ok(Plan {
        span: Span { start_seq, end_seq },
        absorbed,
        collapsed_count,
    })
}

pub(super) fn message_id_at_seq(
    conn: &Connection,
    strand: &str,
    seq: i64,
) -> Result<Option<String>, String> {
    conn.query_row(
        r#"
        SELECT target_id
        FROM r_strand_entries
        WHERE strand_id = ?1 AND strand_seq = ?2 AND target_type = 'message'
        LIMIT 1
        "#,
        params![strand, seq],
        |row| row.get(0),
    )
    .optional()
    .map_err(|error| error.to_string())
}

pub(super) fn entry_text(conn: &Connection, kind: &str, target: &str) -> Result<String, String> {
    Ok(match kind {
        "message" => Database::new(conn)
            .message_record_by_id(target)?
            .map(|message| message.content.rendered())
            .unwrap_or_default(),
        "tool_call" => Database::new(conn)
            .tool_call_by_id(target)?
            .map(|call| format!("[tool_call {}] {}", call.tool, value_text(&call.arguments)))
            .unwrap_or_default(),
        "tool_result" => Database::new(conn)
            .tool_result_by_id(target)?
            .map(|result| match (result.output, result.error) {
                (Some(output), _) => format!("[tool_result] {}", value_text(&output)),
                (None, Some(error)) => format!("[tool_result error] {error}"),
                (None, None) => "[tool_result]".to_string(),
            })
            .unwrap_or_default(),
        "thinking" => Database::new(conn)
            .thinking_span_by_id(target)?
            .and_then(|thinking| thinking.summary)
            .map(|summary| format!("[thinking] {summary}"))
            .unwrap_or_default(),
        _ => String::new(),
    })
}

pub(super) fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

pub(super) fn parse_target_type(value: &str) -> strand::Target {
    match value {
        "compact" => strand::Target::Compact,
        "thinking" => strand::Target::Thinking,
        "tool_call" => strand::Target::ToolCall,
        "tool_result" => strand::Target::ToolResult,
        _ => strand::Target::Message,
    }
}
