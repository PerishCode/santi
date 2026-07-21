pub(crate) use santi_window::Reserved;

use super::SantiStore;
use crate::{Strand, WindowTranscriptEntry};

impl SantiStore {
    pub(crate) fn window_message(
        &self,
        participant_id: &str,
        client_message_id: &str,
    ) -> Result<Option<Reserved>, String> {
        let conn = self.conn.lock().unwrap();
        santi_window::window_message(&conn, participant_id, client_message_id)
    }

    pub(crate) fn labeled_strand(
        &self,
        soul_id: &str,
        label: &str,
    ) -> Result<Option<Strand>, String> {
        let conn = self.conn.lock().unwrap();
        let strand_id = santi_window::labeled_strand_id(&conn, soul_id, label)?;
        drop(conn);
        match strand_id {
            Some(id) => self.strand(&id),
            None => Ok(None),
        }
    }

    pub(crate) fn window_transcript(
        &self,
        strand_id: &str,
        since: i64,
        limit: usize,
    ) -> Result<(Vec<WindowTranscriptEntry>, bool, bool), String> {
        let conn = self.conn.lock().unwrap();
        santi_window::window_transcript(&conn, strand_id, since, limit)
    }
}
