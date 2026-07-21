use santi_protocol::{ReplyEvent, ReplyOutbox};

use super::SantiStore;
use super::db::Database;
use crate::timestamp_now;

impl ReplyOutbox for SantiStore {
    fn pending_replies(&self, limit: usize) -> Result<Vec<ReplyEvent>, String> {
        let conn = self.conn.lock().unwrap();
        Database::new(&conn)
            .pending_reply_payloads(limit)?
            .into_iter()
            .map(|payload| {
                serde_json::from_str::<ReplyEvent>(&payload).map_err(|error| error.to_string())
            })
            .collect()
    }

    fn mark_reply_delivered(&self, id: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        Database::new(&conn).mark_reply_delivered(id, &timestamp_now())
    }
}

impl SantiStore {
    pub(crate) fn connection(&self) -> std::sync::MutexGuard<'_, rusqlite::Connection> {
        self.conn.lock().unwrap()
    }
}
