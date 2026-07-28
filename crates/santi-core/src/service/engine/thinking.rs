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
    pub(in crate::service) async fn tend(&self, progress: Progress<'_>) -> Result<(), String> {
        if let Some(thinking) = progress.current {
            if progress.response.is_some()
                && thinking.response != progress.response
                && let Some(updated) = self
                    .store
                    .update_thinking(
                        &thinking.id,
                        progress.response.as_deref(),
                        None,
                        &crate::now(),
                    )
                    .await?
            {
                *thinking = updated.clone();
                *progress.summary = Some(updated.clone());
                self.publish(
                    progress.strand,
                    stream::Payload::Thinking(crate::thinking::Beat::Updated { thinking: updated }),
                );
            }
            return Ok(());
        }

        let thinking = self
            .store
            .create_thinking(santi_estate::ThinkingDraft {
                tag: &crate::tag("thinking"),
                turn: progress.turn,
                response: progress.response.as_deref(),
                created: &crate::now(),
            })
            .await?;
        self.publish(
            progress.strand,
            stream::Payload::Thinking(crate::thinking::Beat::Created {
                thinking: thinking.clone(),
            }),
        );
        *progress.summary = Some(thinking.clone());
        *progress.current = Some(thinking);
        Ok(())
    }

    pub(in crate::service) async fn summarize(
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
        if let Some(updated) = self
            .store
            .update_thinking(&thinking.id, None, Some(&summary), &crate::now())
            .await?
        {
            *thinking = updated.clone();
            self.publish(
                strand,
                stream::Payload::Thinking(crate::thinking::Beat::Updated { thinking: updated }),
            );
        }
        Ok(())
    }

    pub(in crate::service) async fn conclude(
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
            .complete_thinking(&thinking.id, completion_reason, &crate::now())
            .await?
        {
            self.publish(
                strand,
                stream::Payload::Thinking(crate::thinking::Beat::Completed {
                    thinking: completed,
                }),
            );
        }
        Ok(())
    }

    pub(in crate::service) async fn abandon(
        &self,
        strand: &str,
        current: &mut Option<thinking::Span>,
        error: String,
    ) -> Result<(), String> {
        let Some(thinking) = current.take() else {
            return Ok(());
        };
        if let Some(failed) = self
            .store
            .fail_thinking(&thinking.id, &error, &crate::now())
            .await?
        {
            self.publish(
                strand,
                stream::Payload::Thinking(crate::thinking::Beat::Completed { thinking: failed }),
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
            stream::Payload::Turn(crate::turn::Beat::Active {
                activity: turn::Activity {
                    turn: turn.to_string(),
                    state,
                    response,
                },
            }),
        );
    }
}
