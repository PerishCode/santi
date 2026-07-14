use futures_util::StreamExt;
use santi_provider::{ProviderEvent, ProviderFunctionCall, ProviderRequest, ProviderStream};

use crate::assembly::input::provider_input;
use crate::context::budget::estimate_provider_request;
use crate::service::tools::provider_tools;
use crate::{
    SantiStreamPayload, StrandMessage, ThinkingCompletionReason, ThinkingSpan, TurnActivityState,
};

use super::super::{
    Service, address::Address, notice::Observation, text::delta, timing,
    timing::provider_event_name,
};
use super::budget::Verdict;
use super::failure::{Admission, Failure, Metadata, Operation, Persistence, Stage};

struct Output {
    calls: Vec<ProviderFunctionCall>,
    completed_response_id: Option<String>,
    active_provider_response_id: Option<String>,
    assistant_text: String,
}

struct Driver<'a, 'turn> {
    service: &'a Service,
    address: Address<&'a str>,
    number: usize,
    provider_family: &'a str,
    request_model: &'a str,
    assistant_text: &'a mut String,
    timing: &'a mut timing::Turn<'turn>,
    calls: Vec<ProviderFunctionCall>,
    completed_response_id: Option<String>,
    active_provider_response_id: Option<String>,
    current_thinking_span: Option<ThinkingSpan>,
    summary_thinking_span: Option<ThinkingSpan>,
    reasoning_summary: String,
    round_assistant_text: String,
    saw_sse_event: bool,
}

impl Driver<'_, '_> {
    async fn consume(mut self, mut stream: ProviderStream) -> Result<Output, Failure> {
        while let Some(event) = stream.next().await {
            let Some(event) = self.receive_event(event)? else {
                continue;
            };
            self.record_first_event(&event);
            if self.handle_event(event)? {
                break;
            }
        }
        Ok(Output {
            calls: self.calls,
            completed_response_id: self.completed_response_id,
            active_provider_response_id: self.active_provider_response_id,
            assistant_text: self.round_assistant_text,
        })
    }

    fn receive_event(
        &mut self,
        event: Result<ProviderEvent, String>,
    ) -> Result<Option<ProviderEvent>, Failure> {
        match event {
            Ok(ProviderEvent::StreamTrace(trace)) => {
                self.timing.provider_trace(self.number, trace);
                Ok(None)
            }
            Ok(event) => Ok(Some(event)),
            Err(error) => {
                self.timing.failed(self.number, "sse_event", &error);
                let result = self.service.fail_current_thinking_span(
                    self.address.strand_id,
                    &mut self.current_thinking_span,
                    error.clone(),
                );
                self.runtime(Operation::Persistence(Persistence::Thinking), result)?;
                Err(self.provider_failure(error, Stage::Stream))
            }
        }
    }

    fn record_first_event(&mut self, event: &ProviderEvent) {
        if !self.saw_sse_event {
            self.saw_sse_event = true;
            self.timing
                .first_sse_event(self.number, provider_event_name(event));
        }
    }

    fn handle_event(&mut self, event: ProviderEvent) -> Result<bool, Failure> {
        match event {
            ProviderEvent::StreamTrace(_) => Ok(false),
            ProviderEvent::ResponseStarted {
                provider_response_id,
            }
            | ProviderEvent::ResponseInProgress {
                provider_response_id,
            } => self.response_progress(provider_response_id),
            ProviderEvent::ReasoningSummaryDelta(delta) => {
                self.reasoning_summary.push_str(&delta);
                self.persist_reasoning_summary()?;
                Ok(false)
            }
            ProviderEvent::ReasoningSummaryDone(summary) => {
                self.reasoning_summary = summary;
                self.persist_reasoning_summary()?;
                Ok(false)
            }
            ProviderEvent::TextDelta(delta) => {
                self.text_delta(delta)?;
                Ok(false)
            }
            ProviderEvent::FunctionCallRequested(call) => {
                self.function_call_requested(call)?;
                Ok(false)
            }
            ProviderEvent::Completed {
                provider_response_id,
            } => self.complete(provider_response_id),
            ProviderEvent::Failed(error) => Err(self.failed(error)),
        }
    }

    fn response_progress(&mut self, provider_response_id: Option<String>) -> Result<bool, Failure> {
        self.active_provider_response_id = provider_response_id.clone();
        let result = self
            .service
            .ensure_thinking_span(crate::service::thinking::Progress {
                strand: self.address.strand_id,
                turn: self.address.turn_id,
                current: &mut self.current_thinking_span,
                summary: &mut self.summary_thinking_span,
                response: provider_response_id.clone(),
            });
        self.runtime(Operation::Persistence(Persistence::Thinking), result)?;
        self.service.publish_turn_activity(
            self.address.strand_id,
            self.address.turn_id,
            TurnActivityState::Thinking,
            provider_response_id,
        );
        Ok(false)
    }

    fn persist_reasoning_summary(&mut self) -> Result<(), Failure> {
        let result = self.service.update_thinking_span_summary(
            self.address.strand_id,
            &mut self.summary_thinking_span,
            self.reasoning_summary.clone(),
        );
        self.runtime(Operation::Persistence(Persistence::Thinking), result)
    }

    fn text_delta(&mut self, delta: String) -> Result<(), Failure> {
        let update = delta::Update {
            address: self.address.clone(),
            assistant_text: self.assistant_text,
            round_assistant_text: &mut self.round_assistant_text,
            timing: self.timing,
            round: self.number,
            current_thinking_span: &mut self.current_thinking_span,
            active_provider_response_id: &self.active_provider_response_id,
        };
        let result = self.service.handle_text_delta(delta, update);
        self.runtime(Operation::Persistence(Persistence::Text), result)
    }

    fn function_call_requested(&mut self, call: ProviderFunctionCall) -> Result<(), Failure> {
        self.timing.function_call_requested(self.number, &call.name);
        let result = self.service.complete_current_thinking_span(
            self.address.strand_id,
            &mut self.current_thinking_span,
            ThinkingCompletionReason::ToolCallRequested,
        );
        self.runtime(Operation::Persistence(Persistence::Thinking), result)?;
        self.service.publish_turn_activity(
            self.address.strand_id,
            self.address.turn_id,
            TurnActivityState::CallingTool,
            self.active_provider_response_id.clone(),
        );
        self.calls.push(call);
        Ok(())
    }

    fn complete(&mut self, provider_response_id: Option<String>) -> Result<bool, Failure> {
        self.timing.completed(self.number);
        self.active_provider_response_id = provider_response_id.clone();
        let result = self.service.complete_current_thinking_span(
            self.address.strand_id,
            &mut self.current_thinking_span,
            ThinkingCompletionReason::ProviderCompleted,
        );
        self.runtime(Operation::Persistence(Persistence::Thinking), result)?;
        self.completed_response_id = provider_response_id;
        Ok(true)
    }

    fn failed(&mut self, error: String) -> Failure {
        self.timing.failed(self.number, "provider_response", &error);
        let result = self.service.fail_current_thinking_span(
            self.address.strand_id,
            &mut self.current_thinking_span,
            error.clone(),
        );
        match self.runtime(Operation::Persistence(Persistence::Thinking), result) {
            Ok(()) => self.provider_failure(error, Stage::Response),
            Err(failure) => failure,
        }
    }

    fn runtime<T>(&self, operation: Operation, result: Result<T, String>) -> Result<T, Failure> {
        result.map_err(|error| Failure::runtime(operation, error, self.assistant_text.as_str()))
    }

    fn provider_failure(&self, error: String, stage: Stage) -> Failure {
        Failure::provider(
            error,
            self.assistant_text,
            Metadata {
                provider: self.provider_family.to_string(),
                model: self.request_model.to_string(),
                stage,
                round: self.number,
            },
        )
    }
}

impl Service {
    pub(super) async fn complete_provider_turn(&self, strand_id: String, turn_id: String) {
        match self.run_provider_turn(&strand_id, &turn_id).await {
            Err(failure) => {
                self.fail_background_turn(&strand_id, &turn_id, failure);
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
        self.poke(&strand_id, "strand_send", None, "turn_completion_poke");
        self.resume_after_memory_maintenance(&strand_id);
    }

    fn finalize_turn(
        &self,
        strand_id: &str,
        turn_id: &str,
        last_soul_message: Option<StrandMessage>,
        provider_response_id: Option<String>,
    ) {
        if let Some(message) = last_soul_message.as_ref() {
            self.publish_stream(
                strand_id,
                SantiStreamPayload::MessageCompleted {
                    turn_id: turn_id.to_string(),
                    message: message.clone(),
                },
            );
        }
        let metadata = self.provider.metadata();
        match self.store.complete_turn_reply(
            crate::Completion {
                turn: turn_id,
                sequence: last_soul_message
                    .as_ref()
                    .map(|message| message.relation.strand_seq),
                provider: &metadata.provider,
                model: &metadata.model,
                response: provider_response_id,
            },
            last_soul_message.as_ref(),
        ) {
            Ok(_) => {
                self.dispatch_error_events();
                self.publish_stream(
                    strand_id,
                    SantiStreamPayload::TurnCompleted {
                        turn_id: turn_id.to_string(),
                    },
                );
            }
            Err(error) => self.fail_background_turn(
                strand_id,
                turn_id,
                Failure::runtime(Operation::Persistence(Persistence::Completion), error, ""),
            ),
        }
    }

    async fn run_provider_turn(
        &self,
        strand_id: &str,
        turn_id: &str,
    ) -> Result<(Option<StrandMessage>, Option<String>), Failure> {
        let mut assistant_text = String::new();
        let mut last_soul_message: Option<StrandMessage> = None;
        let mut timing = timing::Turn::new(turn_id);
        let mut round = 0;
        macro_rules! provider_try {
            ($operation:expr, $expr:expr) => {
                match $expr {
                    Ok(value) => value,
                    Err(error) => {
                        return Err(Failure::runtime($operation, error, &assistant_text));
                    }
                }
            };
        }

        let final_response_id = loop {
            let next_round = round + 1;
            if let Some(error) = provider_try!(
                Operation::Admission(Admission::Execution),
                self.admit_execution_round(strand_id, turn_id, next_round)
            ) {
                return Err(Failure::execution_budget(error, &assistant_text));
            }
            round = next_round;
            let input = provider_try!(Operation::Assembly, provider_input(&self.store, strand_id));
            let metadata = self.provider.metadata();
            let provider_family = metadata.provider.to_string();
            let request = ProviderRequest {
                model: metadata.model,
                instructions: Some(provider_try!(
                    Operation::Prompt,
                    self.system_prompt_text(strand_id)
                )),
                input,
                tools: Some(provider_tools()),
                previous_response_id: None,
            };
            let estimate = estimate_provider_request(&request);
            if let Some(error) = provider_try!(
                Operation::Admission(Admission::Context),
                self.open_over_budget_incident(strand_id, turn_id, &request, &estimate)
            ) {
                timing.failed(round, "context_budget", &error.to_string());
                return Err(Failure::context_budget(error));
            }
            timing.request_built(
                round,
                request.input.len(),
                request.instructions.as_ref().map_or(0, |text| text.len()),
            );
            self.observe_provider_input(Observation {
                address: Address { strand_id, turn_id },
                round,
                provider: &provider_family,
                model: &request.model,
                input: &request.input,
                instructions: request.instructions.as_deref(),
            });
            self.publish_turn_activity(strand_id, turn_id, TurnActivityState::Requesting, None);
            let request_model = request.model.clone();
            let stream = match self.provider.stream_response(request).await {
                Ok(stream) => {
                    timing.http_response_started(round);
                    stream
                }
                Err(error) => {
                    timing.failed(round, "http_response", &error);
                    return Err(Failure::provider(
                        error,
                        &assistant_text,
                        Metadata {
                            provider: provider_family.clone(),
                            model: request_model.clone(),
                            stage: Stage::Request,
                            round,
                        },
                    ));
                }
            };
            let Output {
                calls,
                completed_response_id,
                active_provider_response_id,
                assistant_text: round_assistant_text,
            } = Driver {
                service: self,
                address: Address { strand_id, turn_id },
                number: round,
                provider_family: &provider_family,
                request_model: &request_model,
                assistant_text: &mut assistant_text,
                timing: &mut timing,
                calls: Vec::new(),
                completed_response_id: None,
                active_provider_response_id: None,
                current_thinking_span: None,
                summary_thinking_span: None,
                reasoning_summary: String::new(),
                round_assistant_text: String::new(),
                saw_sse_event: false,
            }
            .consume(stream)
            .await?;

            if !round_assistant_text.is_empty() {
                last_soul_message = Some(provider_try!(
                    Operation::Persistence(Persistence::Assistant),
                    self.store
                        .append_soul_assistant_text(strand_id, &round_assistant_text)
                ));
            }

            if calls.is_empty() {
                break completed_response_id;
            }

            let output_limits = match provider_try!(
                Operation::Admission(Admission::Execution),
                self.admit_tool_batch(strand_id, turn_id, round, calls.len())
            ) {
                Verdict::Unbounded => vec![None; calls.len()],
                Verdict::Bounded(limits) => limits.into_iter().map(Some).collect::<Vec<_>>(),
                Verdict::Rejected(error) => {
                    return Err(Failure::execution_budget(*error, &assistant_text));
                }
            };
            timing.tool_outputs_started(round, calls.len());
            let call_count = calls.len();
            for (call, output_limit) in calls.into_iter().zip(output_limits) {
                self.publish_turn_activity(
                    strand_id,
                    turn_id,
                    TurnActivityState::RunningTool,
                    active_provider_response_id.clone(),
                );
                provider_try!(
                    Operation::Tool,
                    self.handle_tool_call(strand_id, turn_id, call, output_limit)
                );
            }
            timing.tool_outputs_completed(round, call_count);
        };

        Ok((last_soul_message, final_response_id))
    }
}
