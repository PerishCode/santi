//! IM layer service methods — the thin seam between the plain IM and the runtime.
//! The IM is conceptually ORTHOGONAL to the runtime (PHASE-08 CONVERGED MODEL v4):
//! inbound reuses the source-less runtime primitive `ingest`, addressing the soul
//! by an `im:<participant>` conversation label; the participant address lives only
//! in the IM envelope (the label + the IM store), never in the runtime.

use crate::{
    IM_LABEL_PREFIX, ImInboxEntry, IngestOutcome, MessageContent, MessageKind, StrandSelector,
};

use super::SantiService;

impl SantiService {
    /// IM inbound: a participant sends to a soul. Registers the (persistent)
    /// participant, then delivers the content as a real conversational turn
    /// (`Text` = user speech, heard by the model as `user`) into the IM
    /// conversation strand `im:<participant>` (find-or-create), waking the soul.
    pub fn im_send(
        &self,
        soul_id: &str,
        participant_id: &str,
        content: &str,
    ) -> Result<IngestOutcome, String> {
        self.store.ensure_im_participant(participant_id, "human")?;
        let label = format!("{IM_LABEL_PREFIX}{participant_id}");
        self.ingest(
            StrandSelector::ByLabel {
                soul_id: soul_id.to_string(),
                label,
            },
            MessageContent::text(content.to_string()),
            MessageKind::Text,
            "strand_send",
        )
    }

    /// IM receive: a participant polls its passive inbox past its cursor `since`
    /// (0 = from the beginning). Read-only, no ack — the caller's high-water `seq`
    /// is the ack.
    pub fn im_poll(&self, participant_id: &str, since: i64) -> Result<Vec<ImInboxEntry>, String> {
        self.store.poll_im_inbox(participant_id, since)
    }
}
