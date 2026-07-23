use super::super::{Service, address::Address, timing};
use crate::{message, stream, thinking, turn};

pub(in crate::service) struct Update<'a, 'turn> {
    pub(in crate::service) address: Address<&'a str>,
    pub(in crate::service) assistant_text: &'a mut String,
    pub(in crate::service) round_assistant_text: &'a mut String,
    pub(in crate::service) timing: &'a timing::Turn<'turn>,
    pub(in crate::service) round: usize,
    pub(in crate::service) current_thinking_span: &'a mut Option<thinking::Span>,
    pub(in crate::service) active_provider_response_id: &'a Option<String>,
}

impl Service {
    pub(in crate::service) fn handle_text_delta(
        &self,
        delta: String,
        update: Update<'_, '_>,
    ) -> Result<(), String> {
        if update.assistant_text.is_empty() {
            update.timing.first_text_delta(update.round);
            self.complete_current_thinking_span(
                update.address.strand,
                update.current_thinking_span,
                thinking::Reason::FirstTextDelta,
            )?;
            self.publish_turn_activity(
                update.address.strand,
                update.address.turn,
                turn::Motion::Generating,
                update.active_provider_response_id.clone(),
            );
        }
        update.assistant_text.push_str(&delta);
        update.round_assistant_text.push_str(&delta);
        self.publish_stream(
            update.address.strand,
            stream::Payload::MessageDelta {
                message: format!("stream_{}", update.address.turn),
                turn: update.address.turn.to_string(),
                role: message::Role::Soul,
                text: delta,
            },
        );
        Ok(())
    }
}
