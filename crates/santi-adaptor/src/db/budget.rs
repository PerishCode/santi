use rusqlite::{OptionalExtension, params};

use super::Database;

impl Database<'_> {
    pub fn pending_inbox_rows(&self, strand: &str) -> Result<Vec<(String, String)>, String> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
                SELECT message_kind, content
                FROM strand_inbox
                WHERE strand_id = ?1
                ORDER BY rowid ASC
                "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![strand], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?;
        rows.map(|row| row.map_err(|error| error.to_string()))
            .collect()
    }

    pub fn current_strand_seq(&self, strand: &str) -> Result<Option<i64>, String> {
        self.conn
            .query_row(
                "SELECT next_seq - 1 FROM strands WHERE id = ?1 LIMIT 1",
                params![strand],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())
    }
}
