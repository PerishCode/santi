use crate::assembly::input::input;
use crate::context::budget::gauged;
use crate::service::flow::budget::Verdict;
use crate::service::flow::failure::{Admission, Failure, Metadata, Operation, Persistence, Stage};
use crate::service::tools::tools;
use crate::service::{Service, address::Address, notice::Observation, timing};
use santi_provider::Request;

use super::*;
use crate::{message, stream, turn};

impl Service {
    pub(in crate::service::flow) async fn complete_provider_turn(
        &self,
        strand: String,
        turn: String,
    ) {
        match self.run_provider_turn(&strand, &turn).await {
            Err(failure) => {
                self.fail_background_turn(&strand, &turn, failure);
            }
            Ok((last, response)) => {
                self.finalize_turn(&strand, &turn, last, response);
            }
        }
        self.noticed(&turn);
        self.poke(&strand, "strand_send", None, "turn_completion_poke");
        self.resume_after_memory_maintenance(&strand);
    }

    fn finalize_turn(
        &self,
        strand: &str,
        turn: &str,
        last: Option<message::Placed>,
        response: Option<String>,
    ) {
        if let Some(message) = last.as_ref() {
            self.publish(
                strand,
                stream::Payload::MessageCompleted {
                    turn: turn.to_string(),
                    message: message.clone(),
                },
            );
        }
        let metadata = self.provider.metadata();
        match self.store.finish(
            crate::Completion {
                turn,
                sequence: last.as_ref().map(|message| message.relation.seq),
                provider: &metadata.provider,
                model: &metadata.model,
                response,
            },
            last.as_ref(),
        ) {
            Ok((_, turned)) => {
                self.dispatched();
                let (label, text) = match turned {
                    Some(event) => (Some(event.label), Some(event.text)),
                    None => (None, None),
                };
                self.publish(
                    strand,
                    stream::Payload::TurnCompleted {
                        turn: turn.to_string(),
                        label,
                        text,
                    },
                );
            }
            Err(error) => self.fail_background_turn(
                strand,
                turn,
                Failure::runtime(Operation::Persistence(Persistence::Completion), error, ""),
            ),
        }
    }

    async fn run_provider_turn(
        &self,
        strand: &str,
        turn: &str,
    ) -> Result<(Option<message::Placed>, Option<String>), Failure> {
        let mut prose = String::new();
        let mut last: Option<message::Placed> = None;
        let mut timing = timing::Turn::new(turn);
        let mut round = 0;
        macro_rules! provider_try {
            ($operation:expr, $expr:expr) => {
                match $expr {
                    Ok(value) => value,
                    Err(error) => {
                        return Err(Failure::runtime($operation, error, &prose));
                    }
                }
            };
        }

        let response = loop {
            let next = round + 1;
            if let Some(error) = provider_try!(
                Operation::Admission(Admission::Execution),
                self.admit_execution_round(strand, turn, next)
            ) {
                return Err(Failure::execution_budget(error, &prose));
            }
            round = next;
            let input = provider_try!(Operation::Assembly, input(&self.store, strand));
            let metadata = self.provider.metadata();
            let family = metadata.provider.to_string();
            let request = Request {
                model: metadata.model,
                instructions: Some(provider_try!(
                    Operation::Prompt,
                    self.system_prompt_text(strand)
                )),
                input,
                tools: Some(tools()),
                previous: None,
            };
            let estimate = gauged(&request);
            if let Some(error) = provider_try!(
                Operation::Admission(Admission::Context),
                self.open_over_budget_incident(strand, turn, &request, &estimate)
            ) {
                timing.failed(round, "context_budget", &error.to_string());
                return Err(Failure::context_budget(error));
            }
            timing.built(
                round,
                request.input.len(),
                request.instructions.as_ref().map_or(0, |text| text.len()),
            );
            self.observed(Observation {
                address: Address { strand, turn },
                round,
                provider: &family,
                model: &request.model,
                input: &request.input,
                instructions: request.instructions.as_deref(),
            });
            self.stirred(strand, turn, turn::Motion::Requesting, None);
            let model = request.model.clone();
            let stream = match self.provider.stream(request).await {
                Ok(stream) => {
                    timing.reached(round);
                    stream
                }
                Err(error) => {
                    timing.failed(round, "http_response", &error);
                    return Err(Failure::provider(
                        error,
                        &prose,
                        Metadata {
                            provider: family.clone(),
                            model: model.clone(),
                            stage: Stage::Request,
                            round,
                        },
                    ));
                }
            };
            let Output {
                calls,
                completed,
                active,
                prose: speech,
            } = Driver {
                service: self,
                address: Address { strand, turn },
                number: round,
                family: &family,
                model: &model,
                prose: &mut prose,
                timing: &mut timing,
                calls: Vec::new(),
                completed: None,
                active: None,
                span: None,
                sketch: None,
                summary: String::new(),
                speech: String::new(),
                seen: false,
            }
            .consume(stream)
            .await?;

            if !speech.is_empty() {
                last = Some(provider_try!(
                    Operation::Persistence(Persistence::Assistant),
                    self.store.append_soul_assistant_text(strand, &speech)
                ));
            }

            if calls.is_empty() {
                break completed;
            }

            let limits = match provider_try!(
                Operation::Admission(Admission::Execution),
                self.admit_tool_batch(strand, turn, round, calls.len())
            ) {
                Verdict::Unbounded => vec![None; calls.len()],
                Verdict::Bounded(limits) => limits.into_iter().map(Some).collect::<Vec<_>>(),
                Verdict::Rejected(error) => {
                    return Err(Failure::execution_budget(*error, &prose));
                }
            };
            timing.outputting(round, calls.len());
            let count = calls.len();
            for (call, output_limit) in calls.into_iter().zip(limits) {
                self.stirred(strand, turn, turn::Motion::Running, active.clone());
                provider_try!(
                    Operation::Tool,
                    self.tooled(strand, turn, call, output_limit)
                );
            }
            timing.outputted(round, count);
        };

        Ok((last, response))
    }
}
