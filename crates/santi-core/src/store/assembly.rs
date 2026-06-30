use rusqlite::params;
use santi_provider::{ProviderHistoricalCall, ProviderMessage};
use serde_json::json;

use super::{
    SantiStore,
    db::{compact_by_id, session_message_to_provider, tool_call_by_id, tool_result_by_id},
};

impl SantiStore {
    /// Project the ordered soul-session timeline into provider messages.
    ///
    /// `tools_through_seq` is the in-flight turn's `base_soul_session_seq`: tool
    /// calls/results at or below it belong to COMPLETED turns and are replayed
    /// here as historical tool messages, while anything above it is the in-flight
    /// turn's trailing roundtrip, which the service still drives through
    /// `function_call_outputs` (preserving the provider's raw item / reasoning
    /// echo). Pass `i64::MAX` to render every tool entry.
    pub fn assembly_input(
        &self,
        soul_session_id: &str,
        tools_through_seq: i64,
    ) -> Result<Vec<ProviderMessage>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                r#"
                SELECT target_type, target_id, soul_session_seq
                FROM r_soul_session_messages
                WHERE soul_session_id = ?1
                ORDER BY soul_session_seq ASC
                "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![soul_session_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        let mut input = Vec::new();
        for row in rows {
            let (target_type, target_id, seq) = row.map_err(|error| error.to_string())?;
            match target_type.as_str() {
                "message" => {
                    if let Some(message) = super::db::message_by_id(&conn, &target_id)?
                        && let Some(provider_message) = session_message_to_provider(&message)
                    {
                        input.push(provider_message);
                    }
                }
                "compact" => {
                    if let Some(compact) = compact_by_id(&conn, &target_id)? {
                        input.push(ProviderMessage::Text {
                            role: "system".to_string(),
                            content: format!(
                                "[compact {}-{}]\n{}",
                                compact.start_session_seq, compact.end_session_seq, compact.summary
                            ),
                        });
                    }
                }
                // Completed-turn tool history is replayed so the soul keeps a
                // record of tools it ran across turns. In-flight calls
                // (seq > tools_through_seq) are left to `function_call_outputs`.
                "tool_call" if seq <= tools_through_seq => {
                    if let Some(tool_call) = tool_call_by_id(&conn, &target_id)? {
                        input.push(ProviderMessage::ToolCalls {
                            calls: vec![ProviderHistoricalCall {
                                call_id: tool_call.id,
                                name: tool_call.tool_name,
                                arguments_raw: serde_json::to_string(&tool_call.arguments)
                                    .map_err(|error| error.to_string())?,
                            }],
                        });
                    }
                }
                "tool_result" if seq <= tools_through_seq => {
                    if let Some(tool_result) = tool_result_by_id(&conn, &target_id)? {
                        let content = serde_json::to_string(&json!({
                            "ok": tool_result.error_text.is_none(),
                            "output": tool_result.output,
                            "error": tool_result.error_text,
                        }))
                        .map_err(|error| error.to_string())?;
                        input.push(ProviderMessage::ToolResult {
                            call_id: tool_result.tool_call_id,
                            content,
                        });
                    }
                }
                "thinking" | "tool_call" | "tool_result" => {}
                _ => {}
            }
        }
        Ok(input)
    }
}
