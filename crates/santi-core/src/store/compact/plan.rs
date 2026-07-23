use crate::store::{db::Database, span::Span};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use super::*;
use crate::{message, strand};

pub(super) fn planned(
    conn: &Connection,
    strand: &str,
    first: &str,
    last: &str,
) -> Result<Plan, String> {
    for (label, id) in [("from", first), ("to", last)] {
        let message = Database::new(conn)
            .record(id)?
            .ok_or_else(|| format!("compact {label} message not found"))?;
        let projected = message.role == message::Role::Soul
            || (message.role == message::Role::System
                && matches!(
                    message.kind,
                    message::Kind::Text | message::Kind::SantiSystem
                ));
        if !projected || message.state != message::State::Fixed {
            return Err(format!(
                "compact {label} boundary must be a fixed projected message"
            ));
        }
    }

    let database = Database::new(conn);
    let from = database
        .seat(strand, first)?
        .ok_or_else(|| "compact from message not in this strand".to_string())?;
    let to = database
        .seat(strand, last)?
        .ok_or_else(|| "compact to message not in this strand".to_string())?;
    if from > to {
        return Err("compact from must not be after to".to_string());
    }

    let mut absorbed = Vec::new();
    for existing in database.compacts(strand)? {
        let (Some(es), Some(ee)) = (
            database.seat(strand, &existing.first)?,
            database.seat(strand, &existing.last)?,
        ) else {
            continue;
        };
        if ee < from || es > to {
            continue;
        }
        if from <= es && ee <= to {
            absorbed.push(existing.id);
            continue;
        }
        return Err("compact range partially overlaps an existing compact".to_string());
    }

    let collapsed: i64 = conn
        .query_row(
            r#"
            SELECT COUNT(*) FROM r_strand_entries
            WHERE strand_id = ?1 AND strand_seq BETWEEN ?2 AND ?3
            "#,
            params![strand, from, to],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;

    Ok(Plan {
        span: Span { from, to },
        absorbed,
        collapsed,
    })
}

pub(super) fn seated(conn: &Connection, strand: &str, seq: i64) -> Result<Option<String>, String> {
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

pub(super) fn entry(conn: &Connection, kind: &str, target: &str) -> Result<String, String> {
    Ok(match kind {
        "message" => Database::new(conn)
            .record(target)?
            .map(|message| message.content.rendered())
            .unwrap_or_default(),
        "tool_call" => Database::new(conn)
            .call(target)?
            .map(|call| format!("[tool_call {}] {}", call.tool, text(&call.arguments)))
            .unwrap_or_default(),
        "tool_result" => Database::new(conn)
            .reply(target)?
            .map(|result| match (result.output, result.error) {
                (Some(output), _) => format!("[tool_result] {}", text(&output)),
                (None, Some(error)) => format!("[tool_result error] {error}"),
                (None, None) => "[tool_result]".to_string(),
            })
            .unwrap_or_default(),
        "thinking" => Database::new(conn)
            .span(target)?
            .and_then(|thinking| thinking.summary)
            .map(|summary| format!("[thinking] {summary}"))
            .unwrap_or_default(),
        _ => String::new(),
    })
}

pub(super) fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

pub(super) fn targeted(value: &str) -> strand::Target {
    match value {
        "compact" => strand::Target::Compact,
        "thinking" => strand::Target::Thinking,
        "tool_call" => strand::Target::ToolCall,
        "tool_result" => strand::Target::ToolResult,
        _ => strand::Target::Message,
    }
}
