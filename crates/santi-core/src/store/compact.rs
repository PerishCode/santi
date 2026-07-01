use rusqlite::{Connection, params};
use serde_json::Value;

use crate::{
    ActorType, CompactExecResponse, CompactQueryEntry, CompactQueryResponse, MessageState,
    StrandTargetType, prefixed_id,
};

use super::{
    SantiStore,
    db::{
        compact_by_id, compacts_for_strand, message_record_by_id, message_seq_in_strand,
        thinking_span_by_id, tool_call_by_id, tool_result_by_id,
    },
};

impl SantiStore {
    /// Create a compact over `[from_message_id, to_message_id]` in a strand's
    /// spine. Pure projection: the spine is never touched; only the compacts
    /// overlay changes (new row + any fully-covered compacts absorbed). Endpoints
    /// must be FIXED user/assistant messages; the range is disjoint-or-full-cover
    /// against existing compacts (partial overlap is a quick fail).
    pub fn create_compact(
        &self,
        strand_id: &str,
        from_message_id: &str,
        to_message_id: &str,
        summary: &str,
    ) -> Result<CompactExecResponse, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;

        // Endpoints must be fixed user/assistant messages (no in-turn sensing:
        // the current turn's working items are not fixed / lie past `to`).
        for (label, id) in [("from", from_message_id), ("to", to_message_id)] {
            let message = message_record_by_id(&tx, id)?
                .ok_or_else(|| format!("compact {label} message not found"))?;
            let is_conversational =
                matches!(message.actor_type, ActorType::Account | ActorType::Soul);
            if !is_conversational || message.state != MessageState::Fixed {
                return Err(format!(
                    "compact {label} boundary must be a fixed user/assistant message"
                ));
            }
        }

        // Resolve to the one axis compaction lives on: strand_seq.
        let from_seq = message_seq_in_strand(&tx, strand_id, from_message_id)?
            .ok_or_else(|| "compact from message not in this strand".to_string())?;
        let to_seq = message_seq_in_strand(&tx, strand_id, to_message_id)?
            .ok_or_else(|| "compact to message not in this strand".to_string())?;
        if from_seq > to_seq {
            return Err("compact from must not be after to".to_string());
        }

        // Overlap policy: disjoint OR full-cover only. Fully-covered compacts are
        // absorbed (dropped, replaced by the new one). Partial overlap → quick fail.
        let mut absorbed = Vec::new();
        for existing in compacts_for_strand(&tx, strand_id)? {
            let (Some(es), Some(ee)) = (
                message_seq_in_strand(&tx, strand_id, &existing.start_message_id)?,
                message_seq_in_strand(&tx, strand_id, &existing.end_message_id)?,
            ) else {
                continue;
            };
            if ee < from_seq || es > to_seq {
                continue; // disjoint
            }
            if from_seq <= es && ee <= to_seq {
                absorbed.push(existing.id);
                continue; // fully covered
            }
            return Err("compact range partially overlaps an existing compact".to_string());
        }

        let collapsed_count: i64 = tx
            .query_row(
                r#"
                SELECT COUNT(*) FROM r_strand_entries
                WHERE strand_id = ?1 AND strand_seq BETWEEN ?2 AND ?3
                "#,
                params![strand_id, from_seq, to_seq],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;

        for id in &absorbed {
            tx.execute("DELETE FROM compacts WHERE id = ?1", params![id])
                .map_err(|error| error.to_string())?;
        }
        let compact_id = prefixed_id("cmp");
        tx.execute(
            r#"
            INSERT INTO compacts (id, strand_id, summary, start_message_id, end_message_id)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![
                compact_id,
                strand_id,
                summary,
                from_message_id,
                to_message_id
            ],
        )
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;

        Ok(CompactExecResponse {
            compact_id,
            start_message_id: from_message_id.to_string(),
            end_message_id: to_message_id.to_string(),
            absorbed,
            collapsed_count,
        })
    }

    /// Expand a compact: read the PRISTINE spine over its `[start, end]` range and
    /// return the raw interleaved entries (message/tool/reasoning), keyword-filtered
    /// and paginated. The spine is compact-unaware, so this always yields originals.
    pub fn compact_query(
        &self,
        compact_id: &str,
        keyword: Option<&str>,
        page_index: i64,
        page_size: i64,
    ) -> Result<Option<CompactQueryResponse>, String> {
        let conn = self.conn.lock().unwrap();
        let Some(compact) = compact_by_id(&conn, compact_id)? else {
            return Ok(None);
        };
        let mut entries = Vec::new();
        if let (Some(from_seq), Some(to_seq)) = (
            message_seq_in_strand(&conn, &compact.strand_id, &compact.start_message_id)?,
            message_seq_in_strand(&conn, &compact.strand_id, &compact.end_message_id)?,
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

/// Render a spine entry to a plain-text view for `compact query`.
fn entry_text(conn: &Connection, target_type: &str, target_id: &str) -> Result<String, String> {
    Ok(match target_type {
        "message" => message_record_by_id(conn, target_id)?
            .map(|message| message.content.content_text())
            .unwrap_or_default(),
        "tool_call" => tool_call_by_id(conn, target_id)?
            .map(|call| {
                format!(
                    "[tool_call {}] {}",
                    call.tool_name,
                    value_text(&call.arguments)
                )
            })
            .unwrap_or_default(),
        "tool_result" => tool_result_by_id(conn, target_id)?
            .map(|result| match (result.output, result.error_text) {
                (Some(output), _) => format!("[tool_result] {}", value_text(&output)),
                (None, Some(error)) => format!("[tool_result error] {error}"),
                (None, None) => "[tool_result]".to_string(),
            })
            .unwrap_or_default(),
        "thinking" => thinking_span_by_id(conn, target_id)?
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
