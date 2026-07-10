use rusqlite::params;
use santi_provider::ProviderItem;
use serde_json::json;

use super::{
    SantiStore,
    db::{
        compacts_for_strand, message_record_by_id, message_seq_in_strand, message_to_provider_item,
        regenerable_replay_material, thinking_span_by_id, tool_call_by_id, tool_result_by_id,
    },
};

impl SantiStore {
    /// Project the soul-strand's assembled view into the provider's typed-item
    /// input: the immutable spine (r_strand_entries) MERGED at read with
    /// this strand's compact overlay. Each compact collapses its covered
    /// `[start,end]` range into one summary item; the spine itself is never
    /// touched (immutable, compact-unaware, fork-shareable). The turn loop
    /// re-derives input from here each round.
    pub fn assembly_input(&self, strand_id: &str) -> Result<Vec<ProviderItem>, String> {
        let conn = self.conn.lock().unwrap();
        assembly_input_in_conn(&conn, strand_id)
    }

    pub(crate) fn assembly_input_preview(
        &self,
        strand_id: &str,
        response: &crate::CompactExecResponse,
        summary: &str,
        metadata: serde_json::Value,
    ) -> Result<Vec<ProviderItem>, String> {
        let conn = self.conn.lock().unwrap();
        let preview = crate::Compact {
            id: response.compact_id.clone(),
            strand_id: strand_id.to_string(),
            summary: summary.to_string(),
            start_message_id: response.start_message_id.clone(),
            end_message_id: response.end_message_id.clone(),
            created_at: None,
            metadata: Some(metadata),
        };
        assembly_input_with_preview(
            &conn,
            strand_id,
            Some(PreviewCompact {
                start_seq: response.start_seq,
                end_seq: response.end_seq,
                absorbed: response.absorbed.as_slice(),
                content: render_compact_for_provider(
                    &preview,
                    CompactRenderRange {
                        start_seq: response.start_seq,
                        end_seq: response.end_seq,
                        collapsed_count: response.collapsed_count,
                    },
                ),
            }),
        )
    }
}

pub(super) fn assembly_input_in_conn(
    conn: &rusqlite::Connection,
    strand_id: &str,
) -> Result<Vec<ProviderItem>, String> {
    assembly_input_with_preview(conn, strand_id, None)
}

struct PreviewCompact<'a> {
    start_seq: i64,
    end_seq: i64,
    absorbed: &'a [String],
    content: String,
}

struct AssemblyOverlay {
    start_seq: i64,
    end_seq: i64,
    content: String,
}

fn assembly_input_with_preview(
    conn: &rusqlite::Connection,
    strand_id: &str,
    preview: Option<PreviewCompact<'_>>,
) -> Result<Vec<ProviderItem>, String> {
    // Resolve the compact overlay to seq ranges, sorted (disjoint by policy).
    let mut overlay: Vec<AssemblyOverlay> = Vec::new();
    for compact in compacts_for_strand(conn, strand_id)? {
        if preview
            .as_ref()
            .is_some_and(|preview| preview.absorbed.iter().any(|id| id == &compact.id))
        {
            continue;
        }
        if let (Some(from_seq), Some(to_seq)) = (
            message_seq_in_strand(conn, strand_id, &compact.start_message_id)?,
            message_seq_in_strand(conn, strand_id, &compact.end_message_id)?,
        ) {
            overlay.push(AssemblyOverlay {
                start_seq: from_seq,
                end_seq: to_seq,
                content: render_compact_for_provider(
                    &compact,
                    CompactRenderRange {
                        start_seq: from_seq,
                        end_seq: to_seq,
                        collapsed_count: to_seq.saturating_sub(from_seq).saturating_add(1),
                    },
                ),
            });
        }
    }
    if let Some(preview) = preview {
        overlay.push(AssemblyOverlay {
            start_seq: preview.start_seq,
            end_seq: preview.end_seq,
            content: preview.content,
        });
    }
    overlay.sort_by_key(|overlay| overlay.start_seq);

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
        .query_map(params![strand_id], |row| {
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
        let (seq, target_type, target_id) = row.map_err(|error| error.to_string())?;
        // Advance past compacts whose range ends before this seq.
        while overlay_index < overlay.len() && overlay[overlay_index].end_seq < seq {
            overlay_index += 1;
            overlay_emitted = false;
        }
        // Covered by a compact → emit its summary once, skip the underlying.
        if overlay_index < overlay.len() && overlay[overlay_index].start_seq <= seq {
            if !overlay_emitted {
                input.push(ProviderItem::Message {
                    role: "system".to_string(),
                    content: overlay[overlay_index].content.clone(),
                });
                overlay_emitted = true;
            }
            continue;
        }
        match target_type.as_str() {
            "message" => {
                if let Some(message) = message_record_by_id(conn, &target_id)?
                    && let Some(item) = message_to_provider_item(&message)
                {
                    input.push(item);
                }
            }
            "thinking" => {
                // Reasoning is a first-class item; adapters currently drop it
                // (DC5). Emit only when there is real summary text.
                if let Some(thinking) = thinking_span_by_id(conn, &target_id)?
                    && let Some(summary) = thinking.summary.filter(|text| !text.trim().is_empty())
                {
                    input.push(ProviderItem::Reasoning {
                        id: thinking.provider_response_id,
                        content: summary,
                    });
                }
            }
            "tool_call" => {
                if let Some(tool_call) = tool_call_by_id(conn, &target_id)? {
                    // The raw wire item is adaptor-owned advisory replay
                    // material, side-stored — the neutral tool_call carries
                    // no provider plumbing. The adaptor validates it and
                    // regenerates from the neutral fields if it is invalid.
                    let (item, item_id) = regenerable_replay_material(conn, &tool_call.id)?;
                    input.push(ProviderItem::FunctionCall {
                        call_id: tool_call.id,
                        name: tool_call.tool_name,
                        arguments_raw: serde_json::to_string(&tool_call.arguments)
                            .map_err(|error| error.to_string())?,
                        item,
                        item_id,
                    });
                }
            }
            "tool_result" => {
                if let Some(tool_result) = tool_result_by_id(conn, &target_id)? {
                    let output = serde_json::to_string(&json!({
                        "ok": tool_result.error_text.is_none(),
                        "output": tool_result.output,
                        "error": tool_result.error_text,
                    }))
                    .map_err(|error| error.to_string())?;
                    input.push(ProviderItem::FunctionCallOutput {
                        call_id: tool_result.tool_call_id,
                        output,
                    });
                }
            }
            _ => {}
        }
    }
    Ok(input)
}

struct CompactRenderRange {
    start_seq: i64,
    end_seq: i64,
    collapsed_count: i64,
}

fn render_compact_for_provider(
    compact: &crate::Compact,
    fallback_range: CompactRenderRange,
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
        .and_then(|metadata| metadata.get("pre_estimate"))
        .and_then(|estimate| estimate.get("total_bytes"))
        .and_then(|value| value.as_i64());
    let post_total = metadata
        .and_then(|metadata| metadata.get("post_estimate"))
        .and_then(|estimate| estimate.get("total_bytes"))
        .and_then(|value| value.as_i64());
    let budget = metadata
        .and_then(|metadata| metadata.get("budget"))
        .and_then(|budget| budget.get("input_budget_bytes"))
        .and_then(|value| value.as_i64());
    let ratio = metadata
        .and_then(|metadata| metadata.get("compression_ratio"))
        .and_then(|value| value.as_f64());
    let header = json!({
        "schema": "santi.compact_projection.visible_header.v1",
        "compact_id": compact.id,
        "operation": metadata_str(metadata, "operation")
            .unwrap_or(if is_capsule { "manual_capsule" } else { "compact_projection" }),
        "declared_source": metadata_str(metadata, "declared_source").unwrap_or("not_declared"),
        "source_trust": metadata_str(metadata, "source_trust")
            .unwrap_or(if is_capsule { "caller_declared" } else { "not_declared" }),
        "covered_message_range": {
            "start_message_id": compact.start_message_id,
            "end_message_id": compact.end_message_id,
        },
        "covered_strand_seq": {
            "start_seq": start_seq.unwrap_or(fallback_range.start_seq),
            "end_seq": end_seq.unwrap_or(fallback_range.end_seq),
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
            "compression_ratio": ratio,
        },
    });
    let header = serde_json::to_string_pretty(&header).unwrap_or_else(|_| header.to_string());
    format!(
        "[compact projection]\n{header}\n[/compact projection]\n<compact_summary>\n{}\n</compact_summary>",
        compact.summary
    )
}

fn metadata_str<'a>(metadata: Option<&'a serde_json::Value>, key: &str) -> Option<&'a str> {
    metadata
        .and_then(|metadata| metadata.get(key))
        .and_then(|value| value.as_str())
}
