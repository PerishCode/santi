use rusqlite::params;

use super::Database;

impl Database<'_> {
    pub fn insert_reply_outbox(
        &self,
        id: &str,
        turn_id: &str,
        payload: &str,
        created_at: &str,
    ) -> Result<(), String> {
        self.conn
            .execute(
                r#"
                INSERT OR IGNORE INTO reply_outbox (id, turn_id, payload, created_at, delivered_at)
                VALUES (?1, ?2, ?3, ?4, NULL)
                "#,
                params![id, turn_id, payload, created_at],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn pending_reply_payloads(&self, limit: usize) -> Result<Vec<String>, String> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
                SELECT payload
                FROM reply_outbox
                WHERE delivered_at IS NULL
                ORDER BY created_at ASC, id ASC
                LIMIT ?1
                "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![limit as i64], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        rows.map(|row| row.map_err(|error| error.to_string()))
            .collect()
    }

    pub fn mark_reply_delivered(&self, id: &str, at: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE reply_outbox SET delivered_at = ?2 WHERE id = ?1 AND delivered_at IS NULL",
                params![id, at],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}
