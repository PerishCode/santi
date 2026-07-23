use crate::{
    SantiStreamPayload, ThinkingCompletionReason, ThinkingSpan, TurnActivity, TurnActivityState,
};

use super::Service;

pub(in crate::service) struct Progress<'a> {
    pub strand: &'a str,
    pub turn: &'a str,
    pub current: &'a mut Option<ThinkingSpan>,
    pub summary: &'a mut Option<ThinkingSpan>,
    pub response: Option<String>,
}

impl Service {
    pub(in crate::service) fn ensure_thinking_span(
        &self,
        progress: Progress<'_>,
    ) -> Result<(), String> {
        if let Some(thinking) = progress.current {
            if progress.response.is_some()
                && thinking.response != progress.response
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

    pub(in crate::service) fn update_thinking_span_summary(
        &self,
        strand: &str,
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
                strand,
                SantiStreamPayload::ThinkingUpdated { thinking: updated },
            );
        }
        Ok(())
    }

    pub(in crate::service) fn complete_current_thinking_span(
        &self,
        strand: &str,
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
                strand,
                SantiStreamPayload::ThinkingCompleted {
                    thinking: completed,
                },
            );
        }
        Ok(())
    }

    pub(in crate::service) fn fail_current_thinking_span(
        &self,
        strand: &str,
        current: &mut Option<ThinkingSpan>,
        error: String,
    ) -> Result<(), String> {
        let Some(thinking) = current.take() else {
            return Ok(());
        };
        if let Some(failed) = self.store.fail_thinking_span(&thinking.id, error)? {
            self.publish_stream(
                strand,
                SantiStreamPayload::ThinkingCompleted { thinking: failed },
            );
        }
        Ok(())
    }

    pub(in crate::service) fn publish_turn_activity(
        &self,
        strand: &str,
        turn: &str,
        state: TurnActivityState,
        response: Option<String>,
    ) {
        self.publish_stream(
            strand,
            SantiStreamPayload::TurnActivity {
                activity: TurnActivity {
                    turn: turn.to_string(),
                    state,
                    response,
                },
            },
        );
    }
}
