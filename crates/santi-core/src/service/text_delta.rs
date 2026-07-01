use crate::{
    ActorType, SantiStreamPayload, ThinkingCompletionReason, ThinkingSpan, TurnActivityState,
};

use super::{SantiService, timing::ProviderTurnTiming};

pub(super) struct TextDeltaUpdate<'a, 'turn> {
    pub(super) strand_id: &'a str,
    pub(super) turn_id: &'a str,
    pub(super) assistant_text: &'a mut String,
    pub(super) round_assistant_text: &'a mut String,
    pub(super) timing: &'a ProviderTurnTiming<'turn>,
    pub(super) round: usize,
    pub(super) current_thinking_span: &'a mut Option<ThinkingSpan>,
    pub(super) active_provider_response_id: &'a Option<String>,
}

impl SantiService {
    pub(super) fn handle_text_delta(
        &self,
        delta: String,
        update: TextDeltaUpdate<'_, '_>,
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
