use rusqlite::params;

use super::Database;

pub struct Queued<'a> {
    pub id: &'a str,
    pub turn: &'a str,
    pub label: &'a str,
    pub payload: &'a str,
    pub created: &'a str,
}

impl Database<'_> {
    pub fn queue(&self, input: Queued<'_>) -> Result<(), String> {
        self.conn
            .execute(
                r#"
                INSERT OR IGNORE INTO turn_outbox (
                  id, turn_id, external_label, payload, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5)
                "#,
                params![
                    input.id,
                    input.turn,
                    input.label,
                    input.payload,
                    input.created
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn since(
        &self,
        after_seq: i64,
        prefix: &str,
        limit: usize,
    ) -> Result<(i64, Vec<(i64, String)>), String> {
        let crest = self
            .conn
            .query_row("SELECT MAX(seq) FROM turn_outbox", [], |row| {
                row.get::<_, Option<i64>>(0)
            })
            .map_err(|error| error.to_string())?
            .unwrap_or(0)
            .max(after_seq);
        let mut stmt = self
            .conn
            .prepare(
                r#"
                SELECT seq, payload
                FROM turn_outbox
                WHERE seq > ?1
                  AND seq <= ?2
                  AND substr(external_label, 1, length(?3)) = ?3
                ORDER BY seq ASC
                LIMIT ?4
                "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![after_seq, crest, prefix, limit as i64 + 1], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?;
        let mut events = rows
            .map(|row| row.map_err(|error| error.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let cursor = if events.len() > limit {
            events.truncate(limit);
            events.last().map(|(seq, _)| *seq).unwrap_or(after_seq)
        } else {
            crest
        };
        Ok((cursor, events))
    }
}
