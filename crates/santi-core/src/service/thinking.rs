use crate::{
    SantiStreamPayload, ThinkingCompletionReason, ThinkingSpan, TurnActivity, TurnActivityState,
};

use super::Service;

pub(super) struct Progress<'a> {
    pub strand: &'a str,
    pub turn: &'a str,
    pub current: &'a mut Option<ThinkingSpan>,
    pub summary: &'a mut Option<ThinkingSpan>,
    pub response: Option<String>,
}

impl Service {
    pub(super) fn ensure_thinking_span(&self, progress: Progress<'_>) -> Result<(), String> {
        if let Some(thinking) = progress.current {
            if progress.response.is_some()
                && thinking.provider_response_id != progress.response
                && let Some(updated) = self
                    .store
                    .update_thinking_span_response(&thinking.id, progress.response)?
            {
                *thinking = updated.clone();
                *progress.summary = Some(updated.clone());
                self.publish_stream(
                    progress.strand,
                    SantiStreamPayload::ThinkingUpdated { thinking: updated },
                );
            }
            return Ok(());
        }

        let thinking = self
            .store
            .append_thinking_span(progress.turn, progress.response)?;
        self.publish_stream(
            progress.strand,
            SantiStreamPayload::ThinkingCreated {
                thinking: thinking.clone(),
            },
        );
        *progress.summary = Some(thinking.clone());
        *progress.current = Some(thinking);
        Ok(())
    }

    pub(super) fn update_thinking_span_summary(
        &self,
        strand_id: &str,
        summary_target: &mut Option<ThinkingSpan>,
        summary: String,
    ) -> Result<(), String> {
        if summary.trim().is_empty() {
            return Ok(());
        }
        let Some(thinking) = summary_target else {
            return Ok(());
        };
        if let Some(updated) = self
            .store
            .update_thinking_span_summary(&thinking.id, summary)?
        {
            *thinking = updated.clone();
            self.publish_stream(
                strand_id,
                SantiStreamPayload::ThinkingUpdated { thinking: updated },
            );
        }
        Ok(())
    }

    pub(super) fn complete_current_thinking_span(
        &self,
        strand_id: &str,
        current: &mut Option<ThinkingSpan>,
        completion_reason: ThinkingCompletionReason,
    ) -> Result<(), String> {
        let Some(thinking) = current.take() else {
            return Ok(());
        };
        if let Some(completed) = self
            .store
            .complete_thinking_span(&thinking.id, completion_reason)?
        {
            self.publish_stream(
                strand_id,
                SantiStreamPayload::ThinkingCompleted {
                    thinking: completed,
                },
            );
        }
        Ok(())
    }

    pub(super) fn fail_current_thinking_span(
        &self,
        strand_id: &str,
        current: &mut Option<ThinkingSpan>,
        error_text: String,
    ) -> Result<(), String> {
        let Some(thinking) = current.take() else {
            return Ok(());
        };
        if let Some(failed) = self.store.fail_thinking_span(&thinking.id, error_text)? {
            self.publish_stream(
                strand_id,
                SantiStreamPayload::ThinkingCompleted { thinking: failed },
            );
        }
        Ok(())
    }

    pub(super) fn publish_turn_activity(
        &self,
        strand_id: &str,
        turn_id: &str,
        state: TurnActivityState,
        provider_response_id: Option<String>,
    ) {
        self.publish_stream(
            strand_id,
            SantiStreamPayload::TurnActivity {
                activity: TurnActivity {
                    turn_id: turn_id.to_string(),
                    state,
                    provider_response_id,
                },
            },
        );
    }
}
