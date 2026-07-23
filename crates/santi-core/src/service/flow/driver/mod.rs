use futures_util::StreamExt;
use santi_provider::{Call, Event, Streaming};

use super::super::{Service, address::Address, text::delta, timing, timing::provider_event_name};
use super::failure::{Failure, Metadata, Operation, Persistence, Stage};
use crate::{thinking, turn};

mod run;

struct Output {
    calls: Vec<Call>,
    completed_response_id: Option<String>,
    active_provider_response_id: Option<String>,
    assistant_text: String,
}

struct Driver<'a, 'turn> {
    service: &'a Service,
    address: Address<&'a str>,
    number: usize,
    family: &'a str,
    request_model: &'a str,
    assistant_text: &'a mut String,
    timing: &'a mut timing::Turn<'turn>,
    calls: Vec<Call>,
    completed_response_id: Option<String>,
    active_provider_response_id: Option<String>,
    current_thinking_span: Option<thinking::Span>,
    summary_thinking_span: Option<thinking::Span>,
    reasoning_summary: String,
    round_assistant_text: String,
    saw_sse_event: bool,
}

impl Driver<'_, '_> {
    async fn consume(mut self, mut stream: Streaming) -> Result<Output, Failure> {
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

    fn receive_event(&mut self, event: Result<Event, String>) -> Result<Option<Event>, Failure> {
        match event {
            Ok(Event::Traced(trace)) => {
                self.timing.provider_trace(self.number, trace);
                Ok(None)
            }
            Ok(event) => Ok(Some(event)),
            Err(error) => {
                self.timing.failed(self.number, "sse_event", &error);
                let result = self.service.fail_current_thinking_span(
                    self.address.strand,
                    &mut self.current_thinking_span,
                    error.clone(),
                );
                self.runtime(Operation::Persistence(Persistence::Thinking), result)?;
                Err(self.provider_failure(error, Stage::Stream))
            }
        }
    }

    fn record_first_event(&mut self, event: &Event) {
        if !self.saw_sse_event {
            self.saw_sse_event = true;
            self.timing
                .first_sse_event(self.number, provider_event_name(event));
        }
    }

    fn handle_event(&mut self, event: Event) -> Result<bool, Failure> {
        match event {
            Event::Traced(_) => Ok(false),
            Event::Started { response } | Event::Working { response } => {
                self.response_progress(response)
            }
            Event::Thinking(delta) => {
                self.reasoning_summary.push_str(&delta);
                self.persist_reasoning_summary()?;
                Ok(false)
            }
            Event::Thought(summary) => {
                self.reasoning_summary = summary;
                self.persist_reasoning_summary()?;
                Ok(false)
            }
            Event::Text(delta) => {
                self.text_delta(delta)?;
                Ok(false)
            }
            Event::Called(call) => {
                self.function_call_requested(call)?;
                Ok(false)
            }
            Event::Completed { response } => self.complete(response),
            Event::Failed(error) => Err(self.failed(error)),
        }
    }

    fn response_progress(&mut self, response: Option<String>) -> Result<bool, Failure> {
        self.active_provider_response_id = response.clone();
        let result = self
            .service
            .ensure_thinking_span(crate::service::thinking::Progress {
                strand: self.address.strand,
                turn: self.address.turn,
                current: &mut self.current_thinking_span,
                summary: &mut self.summary_thinking_span,
                response: response.clone(),
            });
        self.runtime(Operation::Persistence(Persistence::Thinking), result)?;
        self.service.publish_turn_activity(
            self.address.strand,
            self.address.turn,
            turn::Motion::Thinking,
            response,
        );
        Ok(false)
    }

    fn persist_reasoning_summary(&mut self) -> Result<(), Failure> {
        let result = self.service.update_thinking_span_summary(
            self.address.strand,
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

    fn function_call_requested(&mut self, call: Call) -> Result<(), Failure> {
        self.timing.function_call_requested(self.number, &call.name);
        let result = self.service.complete_current_thinking_span(
            self.address.strand,
            &mut self.current_thinking_span,
            thinking::Reason::ToolCallRequested,
        );
        self.runtime(Operation::Persistence(Persistence::Thinking), result)?;
        self.service.publish_turn_activity(
            self.address.strand,
            self.address.turn,
            turn::Motion::Calling,
            self.active_provider_response_id.clone(),
        );
        self.calls.push(call);
        Ok(())
    }

    fn complete(&mut self, response: Option<String>) -> Result<bool, Failure> {
        self.timing.completed(self.number);
        self.active_provider_response_id = response.clone();
        let result = self.service.complete_current_thinking_span(
            self.address.strand,
            &mut self.current_thinking_span,
            thinking::Reason::ProviderCompleted,
        );
        self.runtime(Operation::Persistence(Persistence::Thinking), result)?;
        self.completed_response_id = response;
        Ok(true)
    }

    fn failed(&mut self, error: String) -> Failure {
        self.timing.failed(self.number, "provider_response", &error);
        let result = self.service.fail_current_thinking_span(
            self.address.strand,
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
                provider: self.family.to_string(),
                model: self.request_model.to_string(),
                stage,
                round: self.number,
            },
        )
    }
}
