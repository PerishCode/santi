use super::*;

impl Service {
    pub(super) async fn salvage(&self, strand: &str, turn: &str, held: &Turn, partial: String) {
        if partial.trim().is_empty() {
            return;
        }
        let content = message::Content::text(partial);
        match self
            .store
            .place(santi_estate::MessageDraft {
                tag: &crate::tag("msg"),
                strand: &held.strand,
                actor: message::Role::Soul,
                actor_id: crate::GENESIS,
                kind: message::Kind::Text,
                content: &content,
                state: message::State::Aborted,
                request: false,
                created: &crate::now(),
            })
            .await
        {
            Ok(message) => {
                self.publish(
                    strand,
                    stream::Payload::Message(crate::message::Beat::Created { message }),
                );
            }
            Err(error) => {
                eprintln!("santi: failed to persist partial output for {turn}: {error}");
            }
        }
    }
}
