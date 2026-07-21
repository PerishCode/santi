use rusqlite::{OptionalExtension, params};
use santi_model::{IM_LABEL_PREFIX, ImDeliveryMode, ImInboxEntry};

use super::SantiStore;

pub struct Reply<'a> {
    pub strand: &'a str,
    pub turn: &'a str,
    pub message: Option<&'a str>,
    pub content: &'a str,
    pub mode: ImDeliveryMode,
}

impl SantiStore {
    pub fn ensure_im_participant(&self, id: &str, kind: &str) -> Result<(), String> {
        self.im.ensure_participant(id, kind)
    }

    pub fn enqueue_im_inbox(
        &self,
        participant_id: &str,
        from_ref: Option<&str>,
        content: &str,
    ) -> Result<ImInboxEntry, String> {
        self.im.enqueue_inbox(participant_id, from_ref, content)
    }

    pub fn enqueue_turn_reply(&self, reply: Reply<'_>) -> Result<(ImInboxEntry, bool), String> {
        let participant = self
            .im_participant_for_strand(reply.strand)?
            .ok_or_else(|| format!("strand {} is not an IM conversation", reply.strand))?;
        self.im.enqueue_reply(santi_im::Reply {
            strand: reply.strand,
            turn: reply.turn,
            participant: &participant,
            message: reply.message,
            content: reply.content,
            mode: reply.mode,
        })
    }

    pub fn poll_im_inbox(
        &self,
        participant_id: &str,
        since: i64,
    ) -> Result<Vec<ImInboxEntry>, String> {
        self.im.poll_inbox(participant_id, since)
    }

    pub fn im_participant_for_strand(&self, strand_id: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().unwrap();
        let label: Option<Option<String>> = conn
            .query_row(
                "SELECT external_label FROM strands WHERE id = ?1",
                params![strand_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        Ok(label
            .flatten()
            .and_then(|label| label.strip_prefix(IM_LABEL_PREFIX).map(str::to_string)))
    }

    pub(crate) fn deliver_reply(&self, event: &santi_protocol::ReplyEvent) -> Result<(), String> {
        self.im.deliver_reply(event)
    }
}
