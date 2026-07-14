use crate::store::{ProviderFault, RuntimeFault};
use crate::{
    ActorType, ErrorScope, ErrorSource, MessageContent, MessageIntake, MessageState, SantiError,
    SantiStreamPayload, Turn, catalog, engine,
};

use super::super::Service;

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
    Budget(Admission, Box<SantiError>),
    Runtime(Operation),
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
            Self::Admission(Admission::Context) => "turn.context_admission",
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
    partial_assistant_text: String,
    cause: Cause,
}

impl Failure {
    pub(super) fn runtime(
        operation: Operation,
        error: String,
        partial_assistant_text: &str,
    ) -> Self {
        Self {
            error,
            partial_assistant_text: partial_assistant_text.to_string(),
            cause: Cause::Runtime(operation),
        }
    }

    pub(super) fn provider(
        error: String,
        partial_assistant_text: &str,
        metadata: Metadata,
    ) -> Self {
        Self {
            error,
            partial_assistant_text: partial_assistant_text.to_string(),
            cause: Cause::Provider(metadata),
        }
    }

    pub(super) fn context_budget(error: SantiError) -> Self {
        Self {
            error: error.to_string(),
            partial_assistant_text: String::new(),
            cause: Cause::Budget(Admission::Context, Box::new(error)),
        }
    }

    pub(super) fn execution_budget(error: SantiError, partial_assistant_text: &str) -> Self {
        Self {
            error: error.to_string(),
            partial_assistant_text: partial_assistant_text.to_string(),
            cause: Cause::Budget(Admission::Execution, Box::new(error)),
        }
    }
}

impl Service {
    pub(super) fn fail_background_turn(&self, strand_id: &str, turn_id: &str, failure: Failure) {
        let Failure {
            error,
            partial_assistant_text,
            cause,
        } = failure;
        let persist_budget = |canonical_error: SantiError, budget: &str| match self
            .store
            .fail_turn_with_incident(turn_id, &error, canonical_error.incident_id.as_deref())
        {
            Ok(turn) => (Some(turn), canonical_error),
            Err(persistence_error) => {
                eprintln!(
                    "santi: {budget}-budget turn persistence failed for {turn_id}: {persistence_error}"
                );
                (
                    None,
                    terminal_runtime_error(strand_id, turn_id, persistence_error),
                )
            }
        };
        let (turn, canonical_error) = match cause {
            Cause::Provider(metadata) => {
                self.persist_provider_failure(strand_id, turn_id, &error, metadata)
            }
            Cause::Budget(Admission::Context, error) => persist_budget(*error, "context"),
            Cause::Budget(Admission::Execution, error) => persist_budget(*error, "execution"),
            Cause::Runtime(operation) => {
                self.persist_runtime_failure(strand_id, turn_id, &error, operation)
            }
        };

        if let Some(turn) = turn {
            self.persist_partial_output(strand_id, turn_id, &turn, partial_assistant_text);
        }
        self.dispatch_error_events();
        self.publish_stream(
            strand_id,
            SantiStreamPayload::TurnFailed {
                turn_id: turn_id.to_string(),
                error: Box::new(canonical_error),
            },
        );
    }

    fn persist_provider_failure(
        &self,
        strand_id: &str,
        turn_id: &str,
        error: &str,
        metadata: Metadata,
    ) -> (Option<Turn>, SantiError) {
        match self.store.fail_provider_turn(
            turn_id,
            error,
            ProviderFault {
                provider: &metadata.provider,
                model: &metadata.model,
                stage: metadata.stage.name(),
                operation: metadata.stage.operation(),
                round: metadata.round,
                detail: error,
            },
        ) {
            Ok((turn, error)) => (Some(turn), error),
            Err(persistence_error) => {
                eprintln!(
                    "santi: provider failure incident persistence failed for {turn_id}: {persistence_error}"
                );
                let turn = self.store.fail_turn(turn_id, error).ok();
                (
                    turn,
                    terminal_persistence_error(strand_id, turn_id, persistence_error),
                )
            }
        }
    }

    fn persist_runtime_failure(
        &self,
        strand_id: &str,
        turn_id: &str,
        error: &str,
        operation: Operation,
    ) -> (Option<Turn>, SantiError) {
        match self.store.fail_runtime_turn(
            turn_id,
            error,
            RuntimeFault {
                operation: operation.name(),
                detail: error,
            },
        ) {
            Ok((turn, error)) => (Some(turn), error),
            Err(persistence_error) => {
                eprintln!(
                    "santi: runtime turn failure persistence failed for {turn_id}: {persistence_error}"
                );
                (
                    None,
                    terminal_runtime_error(strand_id, turn_id, persistence_error),
                )
            }
        }
    }

    fn persist_partial_output(
        &self,
        strand_id: &str,
        turn_id: &str,
        turn: &Turn,
        partial_assistant_text: String,
    ) {
        if partial_assistant_text.trim().is_empty() {
            return;
        }
        match self.store.append_message(crate::Draft {
            strand: &turn.strand_id,
            actor: ActorType::Soul,
            id: self.store.default_soul_id(),
            content: MessageContent::text(partial_assistant_text),
            state: MessageState::Aborted,
            intake: MessageIntake::Record,
        }) {
            Ok(message) => {
                let seq = message.strand_message.relation.strand_seq;
                self.publish_stream(
                    strand_id,
                    SantiStreamPayload::MessageCreated {
                        message: message.strand_message,
                    },
                );
                if let Err(error) = self.store.finish_failed_turn_context(turn_id, seq) {
                    eprintln!("santi: failed to finalize partial output for {turn_id}: {error}");
                }
            }
            Err(error) => {
                eprintln!("santi: failed to persist partial output for {turn_id}: {error}");
            }
        }
    }
}

fn terminal_persistence_error(strand_id: &str, turn_id: &str, detail: String) -> SantiError {
    engine().transient(crate::Signal {
        descriptor: catalog::ERROR_ENGINE_PERSISTENCE_FAILED,
        source: ErrorSource::new("santi-core", "provider_turn_failure"),
        scope: Some(ErrorScope::new("strand", strand_id)),
        message: "failed to persist provider failure incident".to_string(),
        context: serde_json::json!({ "turn_id": turn_id, "detail": detail }),
    })
}

fn terminal_runtime_error(strand_id: &str, turn_id: &str, detail: String) -> SantiError {
    engine().transient(crate::Signal {
        descriptor: catalog::INTERNAL,
        source: ErrorSource::new("santi-core", "turn_failure_persistence"),
        scope: Some(ErrorScope::new("strand", strand_id)),
        message: "failed to persist turn failure".to_string(),
        context: serde_json::json!({ "turn_id": turn_id, "detail": detail }),
    })
}
