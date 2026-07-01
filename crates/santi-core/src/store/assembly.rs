use rusqlite::params;
use santi_provider::ProviderItem;
use serde_json::json;

use super::{
    SantiStore,
    db::{
        compacts_for_strand, message_record_by_id, message_seq_in_strand, message_to_provider_item,
        thinking_span_by_id, tool_call_by_id, tool_result_by_id,
    },
};

impl SantiStore {
    /// Project the soul-session's assembled view into the provider's typed-item
    /// input: the immutable spine (r_strand_entries) MERGED at read with
    /// this strand's compact overlay. Each compact collapses its covered
    /// `[start,end]` range into one summary item; the spine itself is never
    /// touched (immutable, compact-unaware, fork-shareable). The turn loop
    /// re-derives input from here each round.
    pub fn assembly_input(&self, strand_id: &str) -> Result<Vec<ProviderItem>, String> {
        let conn = self.conn.lock().unwrap();
        // Resolve the compact overlay to seq ranges, sorted (disjoint by policy).
        let mut overlay: Vec<(i64, i64, crate::Compact)> = Vec::new();
        for compact in compacts_for_strand(&conn, strand_id)? {
            if let (Some(from_seq), Some(to_seq)) = (
                message_seq_in_strand(&conn, strand_id, &compact.start_message_id)?,
                message_seq_in_strand(&conn, strand_id, &compact.end_message_id)?,
            ) {
                overlay.push((from_seq, to_seq, compact));
            }
        }
        overlay.sort_by_key(|(start, _, _)| *start);

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
            while overlay_index < overlay.len() && overlay[overlay_index].1 < seq {
                overlay_index += 1;
                overlay_emitted = false;
            }
            // Covered by a compact → emit its summary once, skip the underlying.
            if overlay_index < overlay.len() && overlay[overlay_index].0 <= seq {
                if !overlay_emitted {
                    let compact = &overlay[overlay_index].2;
                    input.push(ProviderItem::Message {
                        role: "system".to_string(),
                        content: format!(
                            "[compact {} | {} | {}]\n{}",
                            compact.id,
                            compact.start_message_id,
                            compact.end_message_id,
                            compact.summary
                        ),
                    });
                    overlay_emitted = true;
                }
                continue;
            }
            match target_type.as_str() {
                "message" => {
                    if let Some(message) = message_record_by_id(&conn, &target_id)?
                        && let Some(item) = message_to_provider_item(&message)
                    {
                        input.push(item);
                    }
                }
                "thinking" => {
                    // Reasoning is a first-class item; adapters currently drop it
                    // (DC5). Emit only when there is real summary text.
                    if let Some(thinking) = thinking_span_by_id(&conn, &target_id)?
                        && let Some(summary) =
                            thinking.summary.filter(|text| !text.trim().is_empty())
                    {
                        input.push(ProviderItem::Reasoning {
                            id: thinking.provider_response_id,
                            content: summary,
                        });
                    }
                }
                "tool_call" => {
                    if let Some(tool_call) = tool_call_by_id(&conn, &target_id)? {
                        input.push(ProviderItem::FunctionCall {
                            call_id: tool_call.id,
                            name: tool_call.tool_name,
                            arguments_raw: serde_json::to_string(&tool_call.arguments)
                                .map_err(|error| error.to_string())?,
                            item: tool_call.provider_item,
                            item_id: tool_call.item_id,
                        });
                    }
                }
                "tool_result" => {
                    if let Some(tool_result) = tool_result_by_id(&conn, &target_id)? {
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
}
