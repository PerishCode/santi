use crate::TurnEvent;

use super::SantiStore;
use super::db::Database;

impl SantiStore {
    pub fn turn_events_since(
        &self,
        after_seq: i64,
        limit: usize,
    ) -> Result<Vec<(i64, TurnEvent)>, String> {
        let conn = self.conn.lock().unwrap();
        Database::new(&conn)
            .turn_events_since(after_seq, limit)?
            .into_iter()
            .map(|(seq, payload)| {
                serde_json::from_str::<TurnEvent>(&payload)
                    .map(|event| (seq, event))
                    .map_err(|error| error.to_string())
            })
            .collect()
    }
}
