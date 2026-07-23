use crate::store::{
    db::{Database, message_to_provider_item},
    span::Span,
};
use rusqlite::params;
use santi_provider::ProviderItem;
use serde_json::json;

use super::*;

struct Items<'a> {
    db: Database<'a>,
}

pub(super) fn assembly_input_with_preview(
    conn: &rusqlite::Connection,
    strand: &str,
    preview: Option<Preview<'_>>,
) -> Result<Vec<ProviderItem>, String> {
    let mut overlay: Vec<Overlay> = Vec::new();
    let items = Items {
        db: Database::new(conn),
    };
    for compact in items.db.compacts_for_strand(strand)? {
        if preview
            .as_ref()
            .is_some_and(|preview| preview.absorbed.iter().any(|id| id == &compact.id))
        {
            continue;
        }
        if let (Some(from), Some(to)) = (
            items.db.message_seq_in_strand(strand, &compact.first)?,
            items.db.message_seq_in_strand(strand, &compact.last)?,
        ) {
            overlay.push(Overlay {
                span: Span {
                    start_seq: from,
                    end_seq: to,
                },
                content: render_compact_for_provider(
                    &compact,
                    Range {
                        span: Span {
                            start_seq: from,
                            end_seq: to,
                        },
                        collapsed_count: to.saturating_sub(from).saturating_add(1),
                    },
                ),
            });
        }
    }
    if let Some(preview) = preview {
        overlay.push(Overlay {
            span: Span {
                start_seq: preview.span.start_seq,
                end_seq: preview.span.end_seq,
            },
            content: preview.content,
        });
    }
    overlay.sort_by_key(|overlay| overlay.span.start_seq);

    let mut stmt = conn
        .prepare(
            r#"
                SELECT strand_seq, target_type, target_id
                FROM r_strand_entries
                WHERE strand_id = ?1
                ORDER BY strand_seq ASC
                "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![strand], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    let mut input = Vec::new();
    let mut overlay_index = 0usize;
    let mut overlay_emitted = false;
    for row in rows {
        let (seq, kind, target) = row.map_err(|error| error.to_string())?;
        while overlay_index < overlay.len() && overlay[overlay_index].span.end_seq < seq {
            overlay_index += 1;
            overlay_emitted = false;
        }
        if overlay_index < overlay.len() && overlay[overlay_index].span.start_seq <= seq {
            if !overlay_emitted {
                input.push(ProviderItem::Message {
                    role: "system".to_string(),
                    content: overlay[overlay_index].content.clone(),
                });
                overlay_emitted = true;
            }
            continue;
        }
        if let Some(item) = items.provider(&kind, &target)? {
            input.push(item);
        }
    }
    Ok(input)
}

impl Items<'_> {
    fn provider(&self, kind: &str, target: &str) -> Result<Option<ProviderItem>, String> {
        match kind {
            "message" => self.message(target),
            "thinking" => self.thinking(target),
            "tool_call" => self.tool_call(target),
            "tool_result" => self.tool_result(target),
            _ => Ok(None),
        }
    }

    fn message(&self, target: &str) -> Result<Option<ProviderItem>, String> {
        let Some(message) = self.db.message_record_by_id(target)? else {
            return Ok(None);
        };
        Ok(message_to_provider_item(&message))
    }

    fn thinking(&self, target: &str) -> Result<Option<ProviderItem>, String> {
        let Some(thinking) = self.db.thinking_span_by_id(target)? else {
            return Ok(None);
        };
        let Some(content) = thinking.summary.filter(|text| !text.trim().is_empty()) else {
            return Ok(None);
        };
        Ok(Some(ProviderItem::Reasoning {
            id: thinking.response,
            content,
        }))
    }

    fn tool_call(&self, target: &str) -> Result<Option<ProviderItem>, String> {
        let Some(tool_call) = self.db.tool_call_by_id(target)? else {
            return Ok(None);
        };
        let (item, mark) = self.db.regenerable_replay_material(&tool_call.id)?;
        let arguments_raw =
            serde_json::to_string(&tool_call.arguments).map_err(|error| error.to_string())?;
        Ok(Some(ProviderItem::FunctionCall {
            call_id: tool_call.id,
            name: tool_call.tool,
            arguments_raw,
            item,
            mark,
        }))
    }

    fn tool_result(&self, target: &str) -> Result<Option<ProviderItem>, String> {
        let Some(tool_result) = self.db.tool_result_by_id(target)? else {
            return Ok(None);
        };
        let output = serde_json::to_string(&json!({
            "ok": tool_result.error.is_none(),
            "output": tool_result.output,
            "error": tool_result.error,
        }))
        .map_err(|error| error.to_string())?;
        Ok(Some(ProviderItem::FunctionCallOutput {
            call_id: tool_result.call,
            output,
        }))
    }
}

pub(super) fn render_compact_for_provider(
    compact: &crate::compact::Compact,
    fallback_range: Range,
) -> String {
    let metadata = compact.metadata.as_ref();
    let is_capsule = metadata
        .and_then(|metadata| metadata.get("schema"))
        .and_then(|value| value.as_str())
        == Some("santi.compact_capsule.v1");
    let range = metadata.and_then(|metadata| metadata.get("range"));
    let start_seq = range
        .and_then(|range| range.get("start_seq"))
        .and_then(|value| value.as_i64());
    let end_seq = range
        .and_then(|range| range.get("end_seq"))
        .and_then(|value| value.as_i64());
    let collapsed_count = range
        .and_then(|range| range.get("collapsed_count"))
        .and_then(|value| value.as_i64());
    let pre_total = metadata
        .and_then(|metadata| metadata.get("before"))
        .and_then(|estimate| estimate.get("total"))
        .and_then(|value| value.as_i64());
    let post_total = metadata
        .and_then(|metadata| metadata.get("after"))
        .and_then(|estimate| estimate.get("total"))
        .and_then(|value| value.as_i64());
    let budget = metadata
        .and_then(|metadata| metadata.get("budget"))
        .and_then(|budget| budget.get("bytes"))
        .and_then(|value| value.as_i64());
    let ratio = metadata
        .and_then(|metadata| metadata.get("ratio"))
        .and_then(|value| value.as_f64());
    let header = json!({
        "schema": "santi.compact_projection.visible_header.v1",
        "compact": compact.id,
        "operation": metadata_str(metadata, "operation")
            .unwrap_or(if is_capsule { "manual_capsule" } else { "compact_projection" }),
        "declared_source": metadata_str(metadata, "declared_source").unwrap_or("not_declared"),
        "source_trust": metadata_str(metadata, "source_trust")
            .unwrap_or(if is_capsule { "caller_declared" } else { "not_declared" }),
        "covered_message_range": {
            "first": compact.first,
            "last": compact.last,
        },
        "covered_strand_seq": {
            "start_seq": start_seq.unwrap_or(fallback_range.span.start_seq),
            "end_seq": end_seq.unwrap_or(fallback_range.span.end_seq),
        },
        "collapsed_entries": collapsed_count.unwrap_or(fallback_range.collapsed_count),
        "reason": metadata_str(metadata, "reason").unwrap_or("not_declared"),
        "risk": metadata_str(metadata, "risk").unwrap_or("not_declared"),
        "queryability": metadata_str(metadata, "queryability")
            .unwrap_or("original spine remains queryable with compact query"),
        "originals_query": metadata_str(metadata, "originals_query")
            .map(str::to_string)
            .unwrap_or_else(|| format!("santi compact query --compact-id {}", compact.id)),
        "context_estimate": {
            "pre_total_bytes": pre_total,
            "post_total_bytes": post_total,
            "budget_input_bytes": budget,
            "ratio": ratio,
        },
    });
    let header = serde_json::to_string_pretty(&header).unwrap_or_else(|_| header.to_string());
    format!(
        "[compact projection]\n{header}\n[/compact projection]\n<compact_summary>\n{}\n</compact_summary>",
        compact.summary
    )
}

pub(super) fn metadata_str<'a>(
    metadata: Option<&'a serde_json::Value>,
    key: &str,
) -> Option<&'a str> {
    metadata
        .and_then(|metadata| metadata.get(key))
        .and_then(|value| value.as_str())
}
