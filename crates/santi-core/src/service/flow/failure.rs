use crate::{Fault, Ruled, catalog, engine, turn::Turn};
use crate::{message, stream};

use super::super::Service;

mod provider;
mod runtime;
mod salvage;
mod stop;

#[derive(Debug, Clone, Copy)]
pub(super) enum Stage {
    Request,
    Stream,
    Response,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Stream => "stream",
            Self::Response => "response",
        }
    }

    fn operation(self) -> &'static str {
        match self {
            Self::Request => "turn.request",
            Self::Stream => "turn.stream",
            Self::Response => "turn.response",
        }
    }
}

#[derive(Debug)]
pub(super) struct Metadata {
    pub provider: String,
    pub model: String,
    pub stage: Stage,
    pub round: usize,
}

#[derive(Debug)]
enum Cause {
    Provider(Metadata),
    Budget(Admission, Box<Fault>),
    Runtime(Operation),
    Stopped(crate::turn::Cause),
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Operation {
    Assembly,
    Prompt,
    Admission(Admission),
    Persistence(Persistence),
    Tool,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Persistence {
    Thinking,
    Text,
    Assistant,
    Completion,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Admission {
    Context,
    Execution,
}

impl Operation {
    fn name(self) -> &'static str {
        match self {
            Self::Assembly => "turn.assembly_input",
            Self::Prompt => "turn.system_prompt",
            Self::Admission(Admission::Context) => "turn.admission",
            Self::Admission(Admission::Execution) => "turn.execution_budget_admission",
            Self::Persistence(Persistence::Thinking) => "turn.thinking_persistence",
            Self::Persistence(Persistence::Text) => "turn.text_persistence",
            Self::Persistence(Persistence::Assistant) => "turn.assistant_persistence",
            Self::Tool => "turn.tool_execution",
            Self::Persistence(Persistence::Completion) => "turn.completion_persistence",
        }
    }
}

#[derive(Debug)]
pub(super) struct Failure {
    error: String,
    partial: String,
    cause: Cause,
}

impl Failure {
    pub(super) fn runtime(operation: Operation, error: String, partial: &str) -> Self {
        Self {
            error,
            partial: partial.to_string(),
            cause: Cause::Runtime(operation),
        }
    }

    pub(super) fn provider(error: String, partial: &str, metadata: Metadata) -> Self {
        Self {
            error,
            partial: partial.to_string(),
            cause: Cause::Provider(metadata),
        }
    }

    pub(super) fn context(error: Fault) -> Self {
        Self {
            error: error.to_string(),
            partial: String::new(),
            cause: Cause::Budget(Admission::Context, Box::new(error)),
        }
    }

    pub(super) fn execution(error: Fault, partial: &str) -> Self {
        Self {
            error: error.to_string(),
            partial: partial.to_string(),
            cause: Cause::Budget(Admission::Execution, Box::new(error)),
        }
    }

    pub(super) fn stopped(cause: crate::turn::Cause, partial: &str) -> Self {
        Self {
            error: format!("interrupted by {}", cause.encode()),
            partial: partial.to_string(),
            cause: Cause::Stopped(cause),
        }
    }
}

impl Service {
    pub(super) async fn bury(&self, strand: &str, turn: &str, failure: Failure) {
        let Failure {
            error,
            partial,
            cause,
        } = failure;
        let (finished, canonical_error) = match cause {
            Cause::Provider(metadata) => self.misfired(strand, turn, &error, metadata).await,
            Cause::Budget(admission, canonical_error) => {
                self.failed_budget(strand, turn, &error, admission, *canonical_error)
                    .await
            }
            Cause::Runtime(operation) => self.tripped(strand, turn, &error, operation).await,
            Cause::Stopped(cause) => self.interrupted(strand, turn, cause, &error).await,
        };

        if let Some(held) = finished {
            self.salvage(strand, &held.id, &held, partial).await;
        }
        self.dispatched().await;
        self.publish(
            strand,
            stream::Payload::Turn(crate::turn::Beat::Failed {
                turn: turn.to_string(),
                error: Box::new(canonical_error),
            }),
        );
    }

    async fn failed_budget(
        &self,
        strand: &str,
        turn: &str,
        error: &str,
        admission: Admission,
        canonical: Fault,
    ) -> (Option<Turn>, Fault) {
        let persisted = match canonical.incident.as_deref() {
            Some(incident) => {
                self.store
                    .fail_linked(turn, error, incident, &crate::now())
                    .await
            }
            None => self.store.fail_turn(turn, error, &crate::now()).await,
        };
        match persisted {
            Ok(turn) => (Some(turn), canonical),
            Err(detail) => {
                let budget = match admission {
                    Admission::Context => "context",
                    Admission::Execution => "execution",
                };
                eprintln!("santi: {budget}-budget turn persistence failed for {turn}: {detail}");
                (None, unwritten(strand, turn, detail))
            }
        }
    }
}

fn unrecorded(strand: &str, turn: &str, detail: String) -> Fault {
    engine().transient(crate::Signal {
        descriptor: catalog::UNSAVED,
        source: santi_error::Source::new("santi-core", "provider_turn_failure"),
        scope: Some(santi_error::Scope::new("strand", strand)),
        message: "failed to persist provider failure incident".to_string(),
        context: serde_json::json!({ "turn": turn, "detail": detail }),
    })
}

fn unwritten(strand: &str, turn: &str, detail: String) -> Fault {
    engine().transient(crate::Signal {
        descriptor: catalog::INTERNAL,
        source: santi_error::Source::new("santi-core", "turn_failure_persistence"),
        scope: Some(santi_error::Scope::new("strand", strand)),
        message: "failed to persist turn failure".to_string(),
        context: serde_json::json!({ "turn": turn, "detail": detail }),
    })
}
