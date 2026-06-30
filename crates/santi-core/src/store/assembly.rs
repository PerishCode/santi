use rusqlite::params;
use santi_provider::ProviderItem;
use serde_json::json;

use super::{
    SantiStore,
    db::{
        compact_by_id, message_record_by_id, message_to_provider_item, thinking_span_by_id,
        tool_call_by_id, tool_result_by_id,
    },
};

impl SantiStore {
    /// Project the ordered soul-session timeline into the provider's typed-item
    /// input. The timeline is the single source of truth; every item (messages,
    /// reasoning, tool calls, tool results) is replayed in seq order. In-flight
    /// calls are just the latest items — the turn loop re-derives input from here
    /// each round, so there is no turn boundary to special-case.
    pub fn assembly_input(&self, soul_session_id: &str) -> Result<Vec<ProviderItem>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                r#"
                SELECT target_type, target_id
                FROM r_soul_session_messages
                WHERE soul_session_id = ?1
                ORDER BY soul_session_seq ASC
                "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![soul_session_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?;
        let mut input = Vec::new();
        for row in rows {
            let (target_type, target_id) = row.map_err(|error| error.to_string())?;
            match target_type.as_str() {
                "message" => {
                    if let Some(message) = message_record_by_id(&conn, &target_id)?
                        && let Some(item) = message_to_provider_item(&message)
                    {
                        input.push(item);
                    }
                }
                "compact" => {
                    if let Some(compact) = compact_by_id(&conn, &target_id)? {
                        input.push(ProviderItem::Message {
                            role: "system".to_string(),
                            content: format!(
                                "[compact {}-{}]\n{}",
                                compact.start_session_seq, compact.end_session_seq, compact.summary
                            ),
                        });
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
