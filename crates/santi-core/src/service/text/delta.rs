use crate::{
    ActorType, SantiStreamPayload, ThinkingCompletionReason, ThinkingSpan, TurnActivityState,
};

use super::super::{Service, timing};

pub(in crate::service) struct Update<'a, 'turn> {
    pub(in crate::service) strand_id: &'a str,
    pub(in crate::service) turn_id: &'a str,
    pub(in crate::service) assistant_text: &'a mut String,
    pub(in crate::service) round_assistant_text: &'a mut String,
    pub(in crate::service) timing: &'a timing::Turn<'turn>,
    pub(in crate::service) round: usize,
    pub(in crate::service) current_thinking_span: &'a mut Option<ThinkingSpan>,
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
                update.strand_id,
                update.current_thinking_span,
                ThinkingCompletionReason::FirstTextDelta,
            )?;
            self.publish_turn_activity(
                update.strand_id,
                update.turn_id,
                TurnActivityState::Generating,
                update.active_provider_response_id.clone(),
            );
        }
        update.assistant_text.push_str(&delta);
        update.round_assistant_text.push_str(&delta);
        self.publish_stream(
            update.strand_id,
            SantiStreamPayload::MessageDelta {
                message_id: format!("stream_{}", update.turn_id),
                turn_id: update.turn_id.to_string(),
                role: ActorType::Soul,
                text: delta,
            },
        );
        Ok(())
    }
}
