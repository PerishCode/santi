use futures_util::StreamExt;
use santi_provider::{ProviderEvent, ProviderRequest};

use crate::assembly::input::provider_input;
use crate::context_budget::estimate_provider_request;
use crate::service_prompt::provider_tools;
use crate::{
    SantiStreamPayload, StrandMessage, ThinkingCompletionReason, ThinkingSpan, TurnActivityState,
};

use super::super::{
    SantiService,
    runtime_notice::ProviderInputObservation,
    text_delta::TextDeltaUpdate,
    timing::{ProviderTurnTiming, provider_event_name},
};
use super::failure::ProviderTurnFailure;

impl SantiService {
    pub(super) async fn complete_provider_turn(&self, strand_id: String, turn_id: String) {
        match self.run_provider_turn(&strand_id, &turn_id).await {
            Err(failure) => {
                self.fail_background_turn(
                    &strand_id,
                    &turn_id,
                    failure.error,
                    failure.partial_assistant_text,
                    failure.record_failure_message,
                );
            }
            Ok((last_soul_message, provider_response_id)) => {
                self.finalize_turn(
                    &strand_id,
                    &turn_id,
                    last_soul_message,
                    provider_response_id,
                );
            }
        }
        self.drain_runtime_notices(&turn_id);
        // Re-check: a turn is one thread "catching up"; requests that arrived
        // during it (seq past this turn's start) make the strand behind
        // again → drive the next turn now.
        self.poke(&strand_id, "strand_send");
    }

    /// Finalize a completed provider turn. Speech is optional (N6): an empty
    /// turn (no per-round text ever appended) is a valid silent completion, not
    /// a failure. `last_soul_message` is the final per-round entry `run_provider_turn`
    /// appended (if any) — already the operator-visible truth, so completion just
    /// marks the turn done, it does not write anything new.
    fn finalize_turn(
        &self,
        strand_id: &str,
        turn_id: &str,
        last_soul_message: Option<StrandMessage>,
        provider_response_id: Option<String>,
    ) {
        let assistant_seq = last_soul_message.map(|message| {
            let seq = message.relation.strand_seq;
            self.publish_stream(
                strand_id,
                SantiStreamPayload::MessageCompleted {
                    turn_id: turn_id.to_string(),
                    message,
                },
            );
            seq
        });
        match self.store.complete_turn(
            turn_id,
            assistant_seq,
            &self.provider.metadata().provider,
            provider_response_id,
        ) {
            Ok(_) => self.publish_stream(
                strand_id,
                SantiStreamPayload::TurnCompleted {
                    turn_id: turn_id.to_string(),
                },
            ),
            Err(error) => self.fail_background_turn(strand_id, turn_id, error, String::new(), true),
        }
    }

    async fn run_provider_turn(
        &self,
        strand_id: &str,
        turn_id: &str,
    ) -> Result<(Option<StrandMessage>, Option<String>), ProviderTurnFailure> {
        let mut assistant_text = String::new();
        let mut last_soul_message: Option<StrandMessage> = None;
        let mut timing = ProviderTurnTiming::new(turn_id);
        let mut round = 0;
        macro_rules! provider_try {
            ($expr:expr) => {
                match $expr {
                    Ok(value) => value,
                    Err(error) => return Err(ProviderTurnFailure::new(error, &assistant_text)),
                }
            };
        }

        let final_response_id = loop {
            round += 1;
            // The timeline is the single source of truth: each round re-derives
            // input from it, including any tool calls/results just persisted by
            // the previous round (no function_call_outputs side-channel).
            let input = provider_try!(provider_input(&self.store, strand_id));
            let metadata = self.provider.metadata();
            let provider_family = metadata.provider.to_string();
            let request = ProviderRequest {
                model: metadata.model,
                instructions: Some(provider_try!(self.system_prompt_text(strand_id))),
                input,
                tools: Some(provider_tools()),
                previous_response_id: None,
            };
            let estimate = estimate_provider_request(&request);
            if let Some(reason) = provider_try!(
                self.block_over_budget_request(strand_id, turn_id, &request, &estimate)
            ) {
                timing.failed(round, "context_budget", &reason);
                return Err(ProviderTurnFailure::context_budget(reason));
            }
            timing.request_built(
                round,
                request.input.len(),
                request.instructions.as_ref().map_or(0, |text| text.len()),
            );
            self.observe_provider_input(ProviderInputObservation {
                strand_id,
                turn_id,
                round,
                provider: &provider_family,
                model: &request.model,
                input: &request.input,
                instructions: request.instructions.as_deref(),
            });
            self.publish_turn_activity(strand_id, turn_id, TurnActivityState::Requesting, None);
            let mut stream = match self.provider.stream_response(request).await {
                Ok(stream) => {
                    timing.http_response_started(round);
                    stream
                }
                Err(error) => {
                    timing.failed(round, "http_response", &error);
                    return Err(ProviderTurnFailure::new(error, &assistant_text));
                }
            };
            let mut calls = Vec::new();
            let mut completed_response_id = None;
            let mut active_provider_response_id = None;
            let mut current_thinking_span: Option<ThinkingSpan> = None;
            let mut summary_thinking_span: Option<ThinkingSpan> = None;
            let mut reasoning_summary = String::new();
            let mut round_assistant_text = String::new();
            let mut saw_sse_event = false;

            while let Some(event) = stream.next().await {
                let event = match event {
                    Ok(event) => event,
                    Err(error) => {
                        timing.failed(round, "sse_event", &error);
                        provider_try!(self.fail_current_thinking_span(
                            strand_id,
                            &mut current_thinking_span,
                            error.clone(),
                        ));
                        return Err(ProviderTurnFailure::new(error, &assistant_text));
                    }
                };
                if let ProviderEvent::StreamTrace(trace) = event {
                    timing.provider_trace(round, trace);
                    continue;
                }
                if !saw_sse_event {
                    saw_sse_event = true;
                    timing.first_sse_event(round, provider_event_name(&event));
                }
                match event {
                    ProviderEvent::StreamTrace(_) => {}
                    ProviderEvent::ResponseStarted {
                        provider_response_id,
                    }
                    | ProviderEvent::ResponseInProgress {
                        provider_response_id,
                    } => {
                        active_provider_response_id = provider_response_id.clone();
                        provider_try!(self.ensure_thinking_span(
                            strand_id,
                            turn_id,
                            &mut current_thinking_span,
                            &mut summary_thinking_span,
                            provider_response_id.clone(),
                        ));
                        self.publish_turn_activity(
                            strand_id,
                            turn_id,
                            TurnActivityState::Thinking,
                            provider_response_id,
                        );
                    }
                    ProviderEvent::ReasoningSummaryDelta(delta) => {
                        reasoning_summary.push_str(&delta);
                        provider_try!(self.update_thinking_span_summary(
                            strand_id,
                            &mut summary_thinking_span,
                            reasoning_summary.clone(),
                        ));
                    }
                    ProviderEvent::ReasoningSummaryDone(summary) => {
                        reasoning_summary = summary;
                        provider_try!(self.update_thinking_span_summary(
                            strand_id,
                            &mut summary_thinking_span,
                            reasoning_summary.clone(),
                        ));
                    }
                    ProviderEvent::TextDelta(delta) => {
                        let update = TextDeltaUpdate {
                            strand_id,
                            turn_id,
                            assistant_text: &mut assistant_text,
                            round_assistant_text: &mut round_assistant_text,
                            timing: &timing,
                            round,
                            current_thinking_span: &mut current_thinking_span,
                            active_provider_response_id: &active_provider_response_id,
                        };
                        provider_try!(self.handle_text_delta(delta, update));
                    }
                    ProviderEvent::FunctionCallRequested(call) => {
                        timing.function_call_requested(round, &call.name);
                        provider_try!(self.complete_current_thinking_span(
                            strand_id,
                            &mut current_thinking_span,
                            ThinkingCompletionReason::ToolCallRequested,
                        ));
                        self.publish_turn_activity(
                            strand_id,
                            turn_id,
                            TurnActivityState::CallingTool,
                            active_provider_response_id.clone(),
                        );
                        calls.push(call);
                    }
                    ProviderEvent::Completed {
                        provider_response_id,
                    } => {
                        timing.completed(round);
                        active_provider_response_id = provider_response_id.clone();
                        provider_try!(self.complete_current_thinking_span(
                            strand_id,
                            &mut current_thinking_span,
                            ThinkingCompletionReason::ProviderCompleted,
                        ));
                        completed_response_id = provider_response_id;
                        break;
                    }
                    ProviderEvent::Failed(error) => {
                        provider_try!(self.fail_current_thinking_span(
                            strand_id,
                            &mut current_thinking_span,
                            error.clone(),
                        ));
                        return Err(ProviderTurnFailure::new(error, &assistant_text));
                    }
                }
            }

            // Persist this round's assistant text as a timeline item before its
            // tool calls (or as the final item), so the replay timeline stays a
            // faithful interleaved log (DC4b). The lumped strand-visible reply is
            // stored once at turn end.
            if !round_assistant_text.is_empty() {
                last_soul_message = Some(provider_try!(
                    self.store
                        .append_soul_assistant_text(strand_id, &round_assistant_text)
                ));
            }

            if calls.is_empty() {
                break completed_response_id;
            }

            timing.tool_outputs_started(round, calls.len());
            let call_count = calls.len();
            for call in calls {
                self.publish_turn_activity(
                    strand_id,
                    turn_id,
                    TurnActivityState::RunningTool,
                    active_provider_response_id.clone(),
                );
                provider_try!(self.handle_tool_call(strand_id, turn_id, call));
            }
            timing.tool_outputs_completed(round, call_count);
        };

        Ok((last_soul_message, final_response_id))
    }
}
