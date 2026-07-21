use crate::assembly::input::provider_input;
use crate::context::budget::estimate_provider_request;
use crate::service::flow::budget::Verdict;
use crate::service::flow::failure::{Admission, Failure, Metadata, Operation, Persistence, Stage};
use crate::service::tools::provider_tools;
use crate::service::{Service, address::Address, notice::Observation, timing};
use crate::{SantiStreamPayload, StrandMessage, TurnActivityState};
use santi_provider::ProviderRequest;

use super::*;

impl Service {
    pub(in crate::service::flow) async fn complete_provider_turn(
        &self,
        strand_id: String,
        turn_id: String,
    ) {
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
                self.dispatch_replies();
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
