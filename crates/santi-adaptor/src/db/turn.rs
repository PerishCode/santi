use rusqlite::params;

use super::Database;

impl Database<'_> {
    pub fn insert_turn_outbox(
        &self,
        id: &str,
        turn_id: &str,
        payload: &str,
        created_at: &str,
    ) -> Result<(), String> {
        self.conn
            .execute(
                r#"
                INSERT OR IGNORE INTO turn_outbox (id, turn_id, payload, created_at)
                VALUES (?1, ?2, ?3, ?4)
                "#,
                params![id, turn_id, payload, created_at],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn turn_events_since(
        &self,
        after_seq: i64,
        limit: usize,
    ) -> Result<Vec<(i64, String)>, String> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
                SELECT seq, payload
                FROM turn_outbox
                WHERE seq > ?1
                ORDER BY seq ASC
                LIMIT ?2
                "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![after_seq, limit as i64], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?;
        rows.map(|row| row.map_err(|error| error.to_string()))
            .collect()
    }
}
