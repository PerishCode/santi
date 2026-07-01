use crate::{ActorType, MessageContent, MessageIntake, MessageState, SantiStreamPayload};

use super::SantiService;

#[derive(Debug)]
pub(super) struct ProviderTurnFailure {
    pub(super) error: String,
    pub(super) partial_assistant_text: String,
}

impl ProviderTurnFailure {
    pub(super) fn new(error: String, partial_assistant_text: &str) -> Self {
        Self {
            error,
            partial_assistant_text: partial_assistant_text.to_string(),
        }
    }
}

impl SantiService {
    pub(super) fn fail_background_turn(
        &self,
        session_id: &str,
        turn_id: &str,
        error: String,
        partial_assistant_text: String,
    ) {
        let mut last_seen_session_seq = None;
        if let Ok(turn) = self.store.fail_turn(turn_id, &error) {
            if !partial_assistant_text.trim().is_empty()
                && let Ok(message) = self.store.append_message(
                    session_id,
                    ActorType::Soul,
                    self.store.default_soul_id(),
                    MessageContent::text(partial_assistant_text),
                    MessageState::Aborted,
                    MessageIntake::Record,
                )
                && self
                    .store
                    .append_message_ref(&turn.strand_id, &message.session_message.message.id)
                    .is_ok()
            {
                last_seen_session_seq = Some(message.session_message.relation.session_seq);
                self.publish_stream(
                    session_id,
                    SantiStreamPayload::MessageCreated {
                        message: message.session_message,
                    },
                );
            }
            if let Ok(message) = self.store.append_santi_system_message(
                session_id,
                failed_system_message(turn_id),
                MessageIntake::Record,
            ) && self
                .store
                .append_message_ref(&turn.strand_id, &message.session_message.message.id)
                .is_ok()
            {
                last_seen_session_seq = Some(message.session_message.relation.session_seq);
                self.publish_stream(
                    session_id,
                    SantiStreamPayload::MessageCreated {
                        message: message.session_message,
                    },
                );
            }
            if let Some(seq) = last_seen_session_seq {
                let _ = self.store.finish_failed_turn_context(turn_id, seq);
            }
        }
        self.publish_stream(
            session_id,
            SantiStreamPayload::TurnFailed {
                turn_id: turn_id.to_string(),
                error,
            },
        );
    }
}

fn failed_system_message(turn_id: &str) -> MessageContent {
    MessageContent::text(
        [
            "<santi-system>".to_string(),
            "kind: turn_failed".to_string(),
            format!("turn_id: {turn_id}"),
            format!("trace: log://turn/{turn_id}"),
            "summary: Previous response attempt failed before completion.".to_string(),
            "</santi-system>".to_string(),
        ]
        .join("\n"),
    )
}
