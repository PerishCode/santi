use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use crate::{
    ActorType, CompactExecResponse, CompactQueryEntry, CompactQueryResponse, MessageKind,
    MessageState, StrandTargetType, prefixed_id, timestamp_now,
};

use super::{SantiStore, db::Database};

struct Plan {
    start_seq: i64,
    end_seq: i64,
    absorbed: Vec<String>,
    collapsed_count: i64,
}

pub(crate) struct Collapse<'a> {
    pub strand: &'a str,
    pub from: &'a str,
    pub to: &'a str,
    pub summary: &'a str,
    pub metadata: Option<Value>,
}

impl SantiStore {
    pub fn create_compact(
        &self,
        strand_id: &str,
        from_message_id: &str,
        to_message_id: &str,
        summary: &str,
    ) -> Result<CompactExecResponse, String> {
        self.create_compact_with_metadata(Collapse {
            strand: strand_id,
            from: from_message_id,
            to: to_message_id,
            summary,
            metadata: None,
        })
    }

    pub(crate) fn create_compact_with_metadata(
        &self,
        collapse: Collapse<'_>,
    ) -> Result<CompactExecResponse, String> {
        let Collapse {
            strand,
            from,
            to,
            summary,
            metadata,
        } = collapse;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let plan = plan_compact_in_tx(&tx, strand, from, to)?;

        for id in &plan.absorbed {
            tx.execute("DELETE FROM compacts WHERE id = ?1", params![id])
                .map_err(|error| error.to_string())?;
        }
        let compact_id = prefixed_id("cmp");
        let now = timestamp_now();
        let metadata_json = metadata
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        tx.execute(
            r#"
            INSERT INTO compacts (
              id, strand_id, summary, start_message_id, end_message_id, created_at, metadata
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
            params![compact_id, strand, summary, from, to, now, metadata_json],
        )
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;

        Ok(CompactExecResponse {
            compact_id,
            start_message_id: from.to_string(),
            end_message_id: to.to_string(),
            start_seq: plan.start_seq,
            end_seq: plan.end_seq,
            absorbed: plan.absorbed,
            collapsed_count: plan.collapsed_count,
            dry_run: false,
            active_incident_resolved: false,
            pre_estimate: None,
            post_estimate: None,
            compression_ratio: None,
        })
    }

    pub(crate) fn preview_compact(
        &self,
        strand_id: &str,
        from_message_id: &str,
        to_message_id: &str,
    ) -> Result<CompactExecResponse, String> {
        let conn = self.conn.lock().unwrap();
        let plan = plan_compact_in_tx(&conn, strand_id, from_message_id, to_message_id)?;
        Ok(CompactExecResponse {
            compact_id: prefixed_id("cmp_preview"),
            start_message_id: from_message_id.to_string(),
            end_message_id: to_message_id.to_string(),
            start_seq: plan.start_seq,
            end_seq: plan.end_seq,
            absorbed: plan.absorbed,
            collapsed_count: plan.collapsed_count,
            dry_run: true,
            active_incident_resolved: false,
            pre_estimate: None,
            post_estimate: None,
            compression_ratio: None,
        })
    }

    pub(crate) fn message_id_at_seq(
        &self,
        strand_id: &str,
        seq: i64,
    ) -> Result<Option<String>, String> {
        let conn = self.conn.lock().unwrap();
        message_id_at_seq(&conn, strand_id, seq)
    }

    pub(crate) fn update_compact_metadata(
        &self,
        compact_id: &str,
        metadata: Value,
    ) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        let metadata_json = serde_json::to_string(&metadata).map_err(|error| error.to_string())?;
        conn.execute(
            "UPDATE compacts SET metadata = ?2 WHERE id = ?1",
            params![compact_id, metadata_json],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn compact_query(
        &self,
        compact_id: &str,
        keyword: Option<&str>,
        page_index: i64,
        page_size: i64,
    ) -> Result<Option<CompactQueryResponse>, String> {
        let conn = self.conn.lock().unwrap();
        let database = Database::new(&conn);
        let Some(compact) = database.compact_by_id(compact_id)? else {
            return Ok(None);
        };
        let mut entries = Vec::new();
        if let (Some(from_seq), Some(to_seq)) = (
            database.message_seq_in_strand(&compact.strand_id, &compact.start_message_id)?,
            database.message_seq_in_strand(&compact.strand_id, &compact.end_message_id)?,
        ) {
            let needle = keyword
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .map(str::to_lowercase);
            let mut stmt = conn
                .prepare(
                    r#"
                    SELECT strand_seq, target_type, target_id
                    FROM r_strand_entries
                    WHERE strand_id = ?1 AND strand_seq BETWEEN ?2 AND ?3
                    ORDER BY strand_seq ASC
                    "#,
                )
                .map_err(|error| error.to_string())?;
            let rows = stmt
                .query_map(params![compact.strand_id, from_seq, to_seq], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(|error| error.to_string())?;
            for row in rows {
                let (seq, target_type, target_id) = row.map_err(|error| error.to_string())?;
                let text = entry_text(&conn, &target_type, &target_id)?;
                if let Some(needle) = &needle
                    && !text.to_lowercase().contains(needle)
                {
                    continue;
                }
                entries.push(CompactQueryEntry {
                    strand_seq: seq,
                    target_type: parse_target_type(&target_type),
                    target_id,
                    text,
                });
            }
        }

        let total = entries.len() as i64;
        let skip = page_index.max(0).saturating_mul(page_size.max(0)).max(0) as usize;
        let take = page_size.max(0) as usize;
        let entries = entries.into_iter().skip(skip).take(take).collect();
        Ok(Some(CompactQueryResponse {
            compact_id: compact.id,
            start_message_id: compact.start_message_id,
            end_message_id: compact.end_message_id,
            total,
            page_index,
            page_size,
            entries,
        }))
    }
}

fn plan_compact_in_tx(
    conn: &Connection,
    strand_id: &str,
    from_message_id: &str,
    to_message_id: &str,
) -> Result<Plan, String> {
    for (label, id) in [("from", from_message_id), ("to", to_message_id)] {
        let message = Database::new(conn)
            .message_record_by_id(id)?
            .ok_or_else(|| format!("compact {label} message not found"))?;
        let is_projected = message.actor_type == ActorType::Soul
            || (message.actor_type == ActorType::System
                && matches!(
                    message.message_kind,
                    MessageKind::Text | MessageKind::SantiSystem
                ));
        if !is_projected || message.state != MessageState::Fixed {
            return Err(format!(
                "compact {label} boundary must be a fixed projected message"
            ));
        }
    }

    let database = Database::new(conn);
    let start_seq = database
        .message_seq_in_strand(strand_id, from_message_id)?
        .ok_or_else(|| "compact from message not in this strand".to_string())?;
    let end_seq = database
        .message_seq_in_strand(strand_id, to_message_id)?
        .ok_or_else(|| "compact to message not in this strand".to_string())?;
    if start_seq > end_seq {
        return Err("compact from must not be after to".to_string());
    }

    let mut absorbed = Vec::new();
    for existing in database.compacts_for_strand(strand_id)? {
        let (Some(es), Some(ee)) = (
            database.message_seq_in_strand(strand_id, &existing.start_message_id)?,
            database.message_seq_in_strand(strand_id, &existing.end_message_id)?,
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
            params![strand_id, start_seq, end_seq],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;

    Ok(Plan {
        start_seq,
        end_seq,
        absorbed,
        collapsed_count,
    })
}

fn message_id_at_seq(
    conn: &Connection,
    strand_id: &str,
    seq: i64,
) -> Result<Option<String>, String> {
    conn.query_row(
        r#"
        SELECT target_id
        FROM r_strand_entries
        WHERE strand_id = ?1 AND strand_seq = ?2 AND target_type = 'message'
        LIMIT 1
        "#,
        params![strand_id, seq],
        |row| row.get(0),
    )
    .optional()
    .map_err(|error| error.to_string())
}

fn entry_text(conn: &Connection, target_type: &str, target_id: &str) -> Result<String, String> {
    Ok(match target_type {
        "message" => Database::new(conn)
            .message_record_by_id(target_id)?
            .map(|message| message.content.content_text())
            .unwrap_or_default(),
        "tool_call" => Database::new(conn)
            .tool_call_by_id(target_id)?
            .map(|call| {
                format!(
                    "[tool_call {}] {}",
                    call.tool_name,
                    value_text(&call.arguments)
                )
            })
            .unwrap_or_default(),
        "tool_result" => Database::new(conn)
            .tool_result_by_id(target_id)?
            .map(|result| match (result.output, result.error_text) {
                (Some(output), _) => format!("[tool_result] {}", value_text(&output)),
                (None, Some(error)) => format!("[tool_result error] {error}"),
                (None, None) => "[tool_result]".to_string(),
            })
            .unwrap_or_default(),
        "thinking" => Database::new(conn)
            .thinking_span_by_id(target_id)?
            .and_then(|thinking| thinking.summary)
            .map(|summary| format!("[thinking] {summary}"))
            .unwrap_or_default(),
        _ => String::new(),
    })
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn parse_target_type(value: &str) -> StrandTargetType {
    match value {
        "compact" => StrandTargetType::Compact,
        "thinking" => StrandTargetType::Thinking,
        "tool_call" => StrandTargetType::ToolCall,
        "tool_result" => StrandTargetType::ToolResult,
        _ => StrandTargetType::Message,
    }
}
