use super::Service;
use crate::{stream, thinking, turn};

pub(in crate::service) struct Progress<'a> {
    pub strand: &'a str,
    pub turn: &'a str,
    pub current: &'a mut Option<thinking::Span>,
    pub summary: &'a mut Option<thinking::Span>,
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
                && let Some(updated) = self.store.attribute(&thinking.id, progress.response)?
            {
                *thinking = updated.clone();
                *progress.summary = Some(updated.clone());
                self.publish(
                    progress.strand,
                    stream::Payload::ThinkingUpdated { thinking: updated },
                );
            }
            return Ok(());
        }

        let thinking = self
            .store
            .append_thinking_span(progress.turn, progress.response)?;
        self.publish(
            progress.strand,
            stream::Payload::ThinkingCreated {
                thinking: thinking.clone(),
            },
        );
        *progress.summary = Some(thinking.clone());
        *progress.current = Some(thinking);
        Ok(())
    }

    pub(in crate::service) fn summarize(
        &self,
        strand: &str,
        summary_target: &mut Option<thinking::Span>,
        summary: String,
    ) -> Result<(), String> {
        if summary.trim().is_empty() {
            return Ok(());
        }
        let Some(thinking) = summary_target else {
            return Ok(());
        };
        if let Some(updated) = self.store.summarize(&thinking.id, summary)? {
            *thinking = updated.clone();
            self.publish(
                strand,
                stream::Payload::ThinkingUpdated { thinking: updated },
            );
        }
        Ok(())
    }

    pub(in crate::service) fn complete_current_thinking_span(
        &self,
        strand: &str,
        current: &mut Option<thinking::Span>,
        completion_reason: thinking::Reason,
    ) -> Result<(), String> {
        let Some(thinking) = current.take() else {
            return Ok(());
        };
        if let Some(completed) = self
            .store
            .complete_thinking_span(&thinking.id, completion_reason)?
        {
            self.publish(
                strand,
                stream::Payload::ThinkingCompleted {
                    thinking: completed,
                },
            );
        }
        Ok(())
    }

    pub(in crate::service) fn fail_current_thinking_span(
        &self,
        strand: &str,
        current: &mut Option<thinking::Span>,
        error: String,
    ) -> Result<(), String> {
        let Some(thinking) = current.take() else {
            return Ok(());
        };
        if let Some(failed) = self.store.fail_thinking_span(&thinking.id, error)? {
            self.publish(
                strand,
                stream::Payload::ThinkingCompleted { thinking: failed },
            );
        }
        Ok(())
    }

    pub(in crate::service) fn stirred(
        &self,
        strand: &str,
        turn: &str,
        state: turn::Motion,
        response: Option<String>,
    ) {
        self.publish(
            strand,
            stream::Payload::TurnActivity {
                activity: turn::Activity {
                    turn: turn.to_string(),
                    state,
                    response,
                },
            },
        );
    }
}
