use crate::assembly::input::input;
use crate::context::budget::gauged;
use crate::service::flow::budget::Verdict;
use crate::service::flow::failure::{Admission, Failure, Metadata, Operation, Persistence, Stage};
use crate::service::tools::tools;
use crate::service::{Service, address::Address, interrupt::Control, notice::Observation, timing};
use santi_provider::Request;

use super::*;
use crate::{message, stream, turn};

impl Service {
    pub(in crate::service::flow) async fn conduct(
        &self,
        strand: String,
        turn: String,
        control: Control,
    ) {
        match self.run(&strand, &turn, &control).await {
            Err(failure) => {
                self.bury(&strand, &turn, failure);
            }
            Ok((last, response)) => {
                if let Some(cause) = self.halted(&control) {
                    self.bury(&strand, &turn, Failure::stopped(cause, ""));
                } else {
                    self.land(&strand, &turn, last, response);
                }
            }
        }
        self.noticed(&turn);
        self.release(&turn);
        self.poke(&strand, "strand_send", None, "turn_completion_poke");
        self.relieve(&strand);
    }

    fn land(
        &self,
        strand: &str,
        turn: &str,
        last: Option<message::Placed>,
        response: Option<String>,
    ) {
        if let Some(message) = last.as_ref() {
            self.publish(
                strand,
                stream::Payload::Message(crate::message::Beat::Completed {
                    turn: turn.to_string(),
                    message: message.clone(),
                }),
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
                    stream::Payload::Turn(crate::turn::Beat::Completed {
                        turn: turn.to_string(),
                        label,
                        text,
                    }),
                );
            }
            Err(error) => match self.store.stopping(turn) {
                Ok(Some(cause)) => self.bury(strand, turn, Failure::stopped(cause, "")),
                Ok(None) | Err(_) => self.bury(
                    strand,
                    turn,
                    Failure::runtime(Operation::Persistence(Persistence::Completion), error, ""),
                ),
            },
        }
    }

    async fn run(
        &self,
        strand: &str,
        turn: &str,
        control: &Control,
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
                self.readmit(strand, turn, next)
            ) {
                return Err(Failure::execution(error, &prose));
            }
            round = next;
            if let Some(cause) = self.halted(control) {
                return Err(Failure::stopped(cause, &prose));
            }
            let input = provider_try!(Operation::Assembly, input(&self.store, strand));
            let metadata = self.provider.metadata();
            let family = metadata.provider.to_string();
            let request = Request {
                model: metadata.model,
                instructions: Some(provider_try!(Operation::Prompt, self.wording(strand))),
                input,
                tools: Some(tools()),
                previous: None,
            };
            let estimate = gauged(&request);
            if let Some(error) = provider_try!(
                Operation::Admission(Admission::Context),
                self.overdrawn(strand, turn, &request, &estimate)
            ) {
                timing.failed(round, "context_budget", &error.to_string());
                return Err(Failure::context(error));
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
            if let Some(cause) = self.halted(control) {
                return Err(Failure::stopped(cause, &prose));
            }
            self.stirred(strand, turn, turn::Motion::Requesting, None);
            let model = request.model.clone();
            let reached = tokio::select! {
                cause = control.wait() => return Err(Failure::stopped(cause, &prose)),
                reached = self.provider.stream(request) => reached,
            };
            let stream = match reached {
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
            .consume(stream, control)
            .await?;

            if !speech.is_empty() {
                last = Some(provider_try!(
                    Operation::Persistence(Persistence::Assistant),
                    self.store.voice(strand, &speech)
                ));
            }

            if calls.is_empty() {
                break completed;
            }

            let limits = match provider_try!(
                Operation::Admission(Admission::Execution),
                self.judge(strand, turn, round, calls.len())
            ) {
                Verdict::Unbounded => vec![None; calls.len()],
                Verdict::Bounded(limits) => limits.into_iter().map(Some).collect::<Vec<_>>(),
                Verdict::Rejected(error) => {
                    return Err(Failure::execution(*error, &prose));
                }
            };
            timing.outputting(round, calls.len());
            let count = calls.len();
            for (call, output_limit) in calls.into_iter().zip(limits) {
                if let Some(cause) = self.halted(control) {
                    return Err(Failure::stopped(cause, &prose));
                }
                self.stirred(strand, turn, turn::Motion::Running, active.clone());
                let result = self
                    .tooled(Address { strand, turn }, call, output_limit, control)
                    .await;
                if let Some(cause) = self.halted(control) {
                    return Err(Failure::stopped(cause, &prose));
                }
                provider_try!(Operation::Tool, result);
            }
            timing.outputted(round, count);
        };

        Ok((last, response))
    }
}
