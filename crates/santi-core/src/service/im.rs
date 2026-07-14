use crate::{
    IM_LABEL_PREFIX, ImInboxEntry, InboxSource, IngestOutcome, MessageContent, MessageKind,
    StrandSelector,
};

use super::{Service, flow::Ingest};

impl Service {
    pub fn im_send(
        &self,
        soul_id: &str,
        participant_id: &str,
        content: &str,
    ) -> Result<IngestOutcome, String> {
        self.store.ensure_im_participant(participant_id, "human")?;
        let label = format!("{IM_LABEL_PREFIX}{participant_id}");
        self.accept(
            StrandSelector::ByLabel {
                soul_id: soul_id.to_string(),
                label,
            },
            Ingest {
                content: MessageContent::text(content.to_string()),
                kind: MessageKind::Text,
                trigger: "strand_send",
                source: Some(InboxSource::new("im").with_ref(participant_id.to_string())),
            },
        )
    }

    pub fn im_poll(&self, participant_id: &str, since: i64) -> Result<Vec<ImInboxEntry>, String> {
        self.store.poll_im_inbox(participant_id, since)
    }
}
