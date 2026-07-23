use crate::store::{
    db::{Database, item},
    span::Span,
};
use rusqlite::params;
use santi_provider::Item;
use serde_json::json;

use super::*;

struct Items<'a> {
    db: Database<'a>,
}

pub(super) fn previewed(
    conn: &rusqlite::Connection,
    strand: &str,
    preview: Option<Preview<'_>>,
) -> Result<Vec<Item>, String> {
    let mut overlay: Vec<Overlay> = Vec::new();
    let items = Items {
        db: Database::new(conn),
    };
    for compact in items.db.compacts(strand)? {
        if preview
            .as_ref()
            .is_some_and(|preview| preview.absorbed.iter().any(|id| id == &compact.id))
        {
            continue;
        }
        if let (Some(from), Some(to)) = (
            items.db.seat(strand, &compact.first)?,
            items.db.seat(strand, &compact.last)?,
        ) {
            overlay.push(Overlay {
                span: Span { from, to },
                content: condensed(
                    &compact,
                    Range {
                        span: Span { from, to },
                        collapsed: to.saturating_sub(from).saturating_add(1),
                    },
                ),
            });
        }
    }
    if let Some(preview) = preview {
        overlay.push(Overlay {
            span: Span {
                from: preview.span.from,
                to: preview.span.to,
            },
            content: preview.content,
        });
    }
    overlay.sort_by_key(|overlay| overlay.span.from);

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
        while overlay_index < overlay.len() && overlay[overlay_index].span.to < seq {
            overlay_index += 1;
            overlay_emitted = false;
        }
        if overlay_index < overlay.len() && overlay[overlay_index].span.from <= seq {
            if !overlay_emitted {
                input.push(Item::Message {
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
    fn provider(&self, kind: &str, target: &str) -> Result<Option<Item>, String> {
        match kind {
            "message" => self.message(target),
            "thinking" => self.thinking(target),
            "tool_call" => self.call(target),
            "tool_result" => self.tool_result(target),
            _ => Ok(None),
        }
    }

    fn message(&self, target: &str) -> Result<Option<Item>, String> {
        let Some(message) = self.db.record(target)? else {
            return Ok(None);
        };
        Ok(item(&message))
    }

    fn thinking(&self, target: &str) -> Result<Option<Item>, String> {
        let Some(thinking) = self.db.span(target)? else {
            return Ok(None);
        };
        let Some(content) = thinking.summary.filter(|text| !text.trim().is_empty()) else {
            return Ok(None);
        };
        Ok(Some(Item::Reasoning {
            id: thinking.response,
            content,
        }))
    }

    fn call(&self, target: &str) -> Result<Option<Item>, String> {
        let Some(call) = self.db.call(target)? else {
            return Ok(None);
        };
        let (item, mark) = self.db.material(&call.id)?;
        let raw = serde_json::to_string(&call.arguments).map_err(|error| error.to_string())?;
        Ok(Some(Item::Call {
            call: call.id,
            name: call.tool,
            raw,
            item,
            mark,
        }))
    }

    fn tool_result(&self, target: &str) -> Result<Option<Item>, String> {
        let Some(tool_result) = self.db.reply(target)? else {
            return Ok(None);
        };
        let output = serde_json::to_string(&json!({
            "ok": tool_result.error.is_none(),
            "output": tool_result.output,
            "error": tool_result.error,
        }))
        .map_err(|error| error.to_string())?;
        Ok(Some(Item::Output {
            call: tool_result.call,
            output,
        }))
    }
}

pub(super) fn condensed(compact: &crate::compact::Compact, fallback_range: Range) -> String {
    let metadata = compact.metadata.as_ref();
    let capsuled = metadata
        .and_then(|metadata| metadata.get("schema"))
        .and_then(|value| value.as_str())
        == Some("santi.compact_capsule.v1");
    let range = metadata.and_then(|metadata| metadata.get("range"));
    let from = range
        .and_then(|range| range.get("start_seq"))
        .and_then(|value| value.as_i64());
    let to = range
        .and_then(|range| range.get("end_seq"))
        .and_then(|value| value.as_i64());
    let collapsed = range
        .and_then(|range| range.get("collapsed_count"))
        .and_then(|value| value.as_i64());
    let before = metadata
        .and_then(|metadata| metadata.get("before"))
        .and_then(|estimate| estimate.get("total"))
        .and_then(|value| value.as_i64());
    let after = metadata
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
        "operation": noted(metadata, "operation")
            .unwrap_or(if capsuled { "manual_capsule" } else { "compact_projection" }),
        "declared_source": noted(metadata, "declared_source").unwrap_or("not_declared"),
        "source_trust": noted(metadata, "source_trust")
            .unwrap_or(if capsuled { "caller_declared" } else { "not_declared" }),
        "covered_message_range": {
            "first": compact.first,
            "last": compact.last,
        },
        "covered_strand_seq": {
            "start_seq": from.unwrap_or(fallback_range.span.from),
            "end_seq": to.unwrap_or(fallback_range.span.to),
        },
        "collapsed_entries": collapsed.unwrap_or(fallback_range.collapsed),
        "reason": noted(metadata, "reason").unwrap_or("not_declared"),
        "risk": noted(metadata, "risk").unwrap_or("not_declared"),
        "queryability": noted(metadata, "queryability")
            .unwrap_or("original spine remains queryable with compact query"),
        "originals_query": noted(metadata, "originals_query")
            .map(str::to_string)
            .unwrap_or_else(|| format!("santi compact query --compact-id {}", compact.id)),
        "context_estimate": {
            "pre_total_bytes": before,
            "post_total_bytes": after,
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

pub(super) fn noted<'a>(metadata: Option<&'a serde_json::Value>, key: &str) -> Option<&'a str> {
    metadata
        .and_then(|metadata| metadata.get(key))
        .and_then(|value| value.as_str())
}
