use super::Store;
use super::db::Database;
use crate::event;

impl Store {
    pub fn since(
        &self,
        after_seq: i64,
        prefix: &str,
        limit: usize,
    ) -> Result<event::Batch, String> {
        let conn = self.conn.lock().unwrap();
        let (cursor, rows) = Database::new(&conn).since(after_seq, prefix, limit)?;
        let events = rows
            .into_iter()
            .map(|(_, payload)| {
                serde_json::from_str::<event::Event>(&payload).map_err(|error| error.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(event::Batch { cursor, events })
    }
}
