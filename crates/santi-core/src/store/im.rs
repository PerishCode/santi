pub use santi_im::{Reply, deliveries_for_receipt_in};
use santi_model::ImInboxEntry;

use super::SantiStore;

impl SantiStore {
    pub fn ensure_im_participant(&self, id: &str, kind: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        santi_im::ensure_participant(&conn, id, kind)
    }

    pub fn enqueue_im_inbox(
        &self,
        participant_id: &str,
        from_ref: Option<&str>,
        content: &str,
    ) -> Result<ImInboxEntry, String> {
        let conn = self.conn.lock().unwrap();
        santi_im::enqueue_inbox(&conn, participant_id, from_ref, content)
    }

    pub fn enqueue_turn_reply(&self, reply: Reply<'_>) -> Result<(ImInboxEntry, bool), String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let outcome = santi_im::enqueue_turn_in(&tx, reply, &santi_model::timestamp_now())?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(outcome)
    }

    pub fn poll_im_inbox(
        &self,
        participant_id: &str,
        since: i64,
    ) -> Result<Vec<ImInboxEntry>, String> {
        let conn = self.conn.lock().unwrap();
        santi_im::poll_inbox(&conn, participant_id, since)
    }

    pub fn im_participant_for_strand(&self, strand_id: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().unwrap();
        santi_im::participant_for_strand(&conn, strand_id)
    }
}
