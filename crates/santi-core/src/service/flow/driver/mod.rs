use futures_util::StreamExt;
use santi_provider::{Call, Event, Streaming};

use super::super::{Service, address::Address, text::delta, timing, timing::named};
use super::failure::{Failure, Metadata, Operation, Persistence, Stage};
use crate::service::interrupt::Control;
use crate::{thinking, turn};

mod run;

struct Output {
    calls: Vec<Call>,
    completed: Option<String>,
    active: Option<String>,
    prose: String,
}

struct Driver<'a, 'turn> {
    service: &'a Service,
    address: Address<&'a str>,
    number: usize,
    family: &'a str,
    model: &'a str,
    prose: &'a mut String,
    timing: &'a mut timing::Turn<'turn>,
    calls: Vec<Call>,
    completed: Option<String>,
    active: Option<String>,
    span: Option<thinking::Span>,
    sketch: Option<thinking::Span>,
    summary: String,
    speech: String,
    seen: bool,
}

impl Driver<'_, '_> {
    async fn consume(
        mut self,
        mut stream: Streaming,
        control: &Control,
    ) -> Result<Output, Failure> {
        loop {
            let event = tokio::select! {
                cause = control.wait() => return Err(Failure::stopped(cause, self.prose)),
                event = stream.next() => event,
            };
            let Some(event) = event else {
                break;
            };
            let Some(event) = self.received(event)? else {
                continue;
            };
            self.recorded(&event);
            if self.handled(event)? {
                break;
            }
        }
        Ok(Output {
            calls: self.calls,
            completed: self.completed,
            active: self.active,
            prose: self.speech,
        })
    }

    fn received(&mut self, event: Result<Event, String>) -> Result<Option<Event>, Failure> {
        match event {
            Ok(Event::Traced(trace)) => {
                self.timing.traced(self.number, trace);
                Ok(None)
            }
            Ok(event) => Ok(Some(event)),
            Err(error) => {
                self.timing.failed(self.number, "sse_event", &error);
                let result =
                    self.service
                        .abandon(self.address.strand, &mut self.span, error.clone());
                self.runtime(Operation::Persistence(Persistence::Thinking), result)?;
                Err(self.faulted(error, Stage::Stream))
            }
        }
    }

    fn recorded(&mut self, event: &Event) {
        if !self.seen {
            self.seen = true;
            self.timing.first(self.number, named(event));
        }
    }

    fn handled(&mut self, event: Event) -> Result<bool, Failure> {
        match event {
            Event::Traced(_) => Ok(false),
            Event::Started { response } | Event::Working { response } => self.progressed(response),
            Event::Thinking(delta) => {
                self.summary.push_str(&delta);
                self.sketched()?;
                Ok(false)
            }
            Event::Thought(summary) => {
                self.summary = summary;
                self.sketched()?;
                Ok(false)
            }
            Event::Text(delta) => {
                self.delta(delta)?;
                Ok(false)
            }
            Event::Called(call) => {
                self.called(call)?;
                Ok(false)
            }
            Event::Completed { response } => self.complete(response),
            Event::Failed(error) => Err(self.failed(error)),
        }
    }

    fn progressed(&mut self, response: Option<String>) -> Result<bool, Failure> {
        self.active = response.clone();
        let result = self.service.tend(crate::service::thinking::Progress {
            strand: self.address.strand,
            turn: self.address.turn,
            current: &mut self.span,
            summary: &mut self.sketch,
            response: response.clone(),
        });
        self.runtime(Operation::Persistence(Persistence::Thinking), result)?;
        self.service.stirred(
            self.address.strand,
            self.address.turn,
            turn::Motion::Thinking,
            response,
        );
        Ok(false)
    }

    fn sketched(&mut self) -> Result<(), Failure> {
        let result =
            self.service
                .summarize(self.address.strand, &mut self.sketch, self.summary.clone());
        self.runtime(Operation::Persistence(Persistence::Thinking), result)
    }

    fn delta(&mut self, delta: String) -> Result<(), Failure> {
        let update = delta::Update {
            address: self.address.clone(),
            prose: self.prose,
            speech: &mut self.speech,
            timing: self.timing,
            round: self.number,
            span: &mut self.span,
            active: &self.active,
        };
        let result = self.service.spoken(delta, update);
        self.runtime(Operation::Persistence(Persistence::Text), result)
    }

    fn called(&mut self, call: Call) -> Result<(), Failure> {
        self.timing.called(self.number, &call.name);
        let result = self.service.conclude(
            self.address.strand,
            &mut self.span,
            thinking::Reason::Called,
        );
        self.runtime(Operation::Persistence(Persistence::Thinking), result)?;
        self.service.stirred(
            self.address.strand,
            self.address.turn,
            turn::Motion::Calling,
            self.active.clone(),
        );
        self.calls.push(call);
        Ok(())
    }

    fn complete(&mut self, response: Option<String>) -> Result<bool, Failure> {
        self.timing.completed(self.number);
        self.active = response.clone();
        let result = self.service.conclude(
            self.address.strand,
            &mut self.span,
            thinking::Reason::Finished,
        );
        self.runtime(Operation::Persistence(Persistence::Thinking), result)?;
        self.completed = response;
        Ok(true)
    }

    fn failed(&mut self, error: String) -> Failure {
        self.timing.failed(self.number, "provider_response", &error);
        let result = self
            .service
            .abandon(self.address.strand, &mut self.span, error.clone());
        match self.runtime(Operation::Persistence(Persistence::Thinking), result) {
            Ok(()) => self.faulted(error, Stage::Response),
            Err(failure) => failure,
        }
    }

    fn runtime<T>(&self, operation: Operation, result: Result<T, String>) -> Result<T, Failure> {
        result.map_err(|error| Failure::runtime(operation, error, self.prose.as_str()))
    }

    fn faulted(&self, error: String, stage: Stage) -> Failure {
        Failure::provider(
            error,
            self.prose,
            Metadata {
                provider: self.family.to_string(),
                model: self.model.to_string(),
                stage,
                round: self.number,
            },
        )
    }
}
