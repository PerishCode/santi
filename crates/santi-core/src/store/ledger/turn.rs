use crate::{TurnEvent, TurnEventBatch};

use super::SantiStore;
use super::db::Database;

impl SantiStore {
    pub fn turn_events_since(
        &self,
        after_seq: i64,
        prefix: &str,
        limit: usize,
    ) -> Result<TurnEventBatch, String> {
        let conn = self.conn.lock().unwrap();
        let (cursor, rows) = Database::new(&conn).turn_events_since(after_seq, prefix, limit)?;
        let events = rows
            .into_iter()
            .map(|(_, payload)| {
                serde_json::from_str::<TurnEvent>(&payload).map_err(|error| error.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(TurnEventBatch { cursor, events })
    }
}
