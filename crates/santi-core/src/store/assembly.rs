use rusqlite::params;

use super::{
    SantiStore,
    db::{compact_by_id, session_message_to_provider},
};

impl SantiStore {
    pub fn assembly_input(
        &self,
        soul_session_id: &str,
    ) -> Result<Vec<crate::ProviderInputMessage>, String> {
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
                    if let Some(message) = super::db::message_by_id(&conn, &target_id)?
                        && let Some(provider_message) = session_message_to_provider(&message)
                    {
                        input.push(provider_message);
                    }
                }
                "compact" => {
                    if let Some(compact) = compact_by_id(&conn, &target_id)? {
                        input.push(crate::ProviderInputMessage {
                            role: "system".to_string(),
                            content: format!(
                                "[compact {}-{}]\n{}",
                                compact.start_session_seq, compact.end_session_seq, compact.summary
                            ),
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
