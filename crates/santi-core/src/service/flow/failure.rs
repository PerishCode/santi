use crate::store::{ProviderFailureContext, RuntimeFailureContext};
use crate::{
    ActorType, ErrorScope, ErrorSource, MessageContent, MessageIntake, MessageState, SantiError,
    SantiStreamPayload, Turn, catalog, engine,
};

use super::super::SantiService;

#[derive(Debug, Clone, Copy)]
pub(super) enum ProviderFailureStage {
    Request,
    Stream,
    Response,
}

impl ProviderFailureStage {
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
struct ProviderFailureMetadata {
    provider: String,
    model: String,
    stage: ProviderFailureStage,
    round: usize,
}

#[derive(Debug)]
enum ProviderTurnFailureCause {
    Provider(ProviderFailureMetadata),
    ContextBudget(SantiError),
    ExecutionBudget(SantiError),
    Runtime(RuntimeTurnFailureOperation),
}

#[derive(Debug, Clone, Copy)]
pub(super) enum RuntimeTurnFailureOperation {
    AssemblyInput,
    SystemPrompt,
    ContextAdmission,
    ExecutionBudgetAdmission,
    ThinkingPersistence,
    TextPersistence,
    AssistantPersistence,
    ToolExecution,
    CompletionPersistence,
}

impl RuntimeTurnFailureOperation {
    fn name(self) -> &'static str {
        match self {
            Self::AssemblyInput => "turn.assembly_input",
            Self::SystemPrompt => "turn.system_prompt",
            Self::ContextAdmission => "turn.context_admission",
            Self::ExecutionBudgetAdmission => "turn.execution_budget_admission",
            Self::ThinkingPersistence => "turn.thinking_persistence",
            Self::TextPersistence => "turn.text_persistence",
            Self::AssistantPersistence => "turn.assistant_persistence",
            Self::ToolExecution => "turn.tool_execution",
            Self::CompletionPersistence => "turn.completion_persistence",
        }
    }
}

#[derive(Debug)]
pub(super) struct ProviderTurnFailure {
    error: String,
    partial_assistant_text: String,
    cause: ProviderTurnFailureCause,
}

impl ProviderTurnFailure {
    pub(super) fn runtime(
        operation: RuntimeTurnFailureOperation,
        error: String,
        partial_assistant_text: &str,
    ) -> Self {
        Self {
            error,
            partial_assistant_text: partial_assistant_text.to_string(),
            cause: ProviderTurnFailureCause::Runtime(operation),
        }
    }

    pub(super) fn provider(
        error: String,
        partial_assistant_text: &str,
        provider: &str,
        model: &str,
        stage: ProviderFailureStage,
        round: usize,
    ) -> Self {
        Self {
            error,
            partial_assistant_text: partial_assistant_text.to_string(),
            cause: ProviderTurnFailureCause::Provider(ProviderFailureMetadata {
                provider: provider.to_string(),
                model: model.to_string(),
                stage,
                round,
            }),
        }
    }

    pub(super) fn context_budget(error: SantiError) -> Self {
        Self {
            error: error.to_string(),
            partial_assistant_text: String::new(),
            cause: ProviderTurnFailureCause::ContextBudget(error),
        }
    }

    pub(super) fn execution_budget(error: SantiError, partial_assistant_text: &str) -> Self {
        Self {
            error: error.to_string(),
            partial_assistant_text: partial_assistant_text.to_string(),
            cause: ProviderTurnFailureCause::ExecutionBudget(error),
        }
    }
}

impl SantiService {
    pub(super) fn fail_background_turn(
        &self,
        strand_id: &str,
        turn_id: &str,
        failure: ProviderTurnFailure,
    ) {
        let ProviderTurnFailure {
            error,
            partial_assistant_text,
            cause,
        } = failure;
        let (turn, canonical_error) = match cause {
            ProviderTurnFailureCause::Provider(metadata) => {
                self.persist_provider_failure(strand_id, turn_id, &error, metadata)
            }
            ProviderTurnFailureCause::ContextBudget(canonical_error) => {
                match self.store.fail_turn_with_incident(
                    turn_id,
                    &error,
                    canonical_error.incident_id.as_deref(),
                ) {
                    Ok(turn) => (Some(turn), canonical_error),
                    Err(persistence_error) => {
                        eprintln!(
                            "santi: context-budget turn persistence failed for {turn_id}: {persistence_error}"
                        );
                        (
                            None,
                            terminal_runtime_error(strand_id, turn_id, persistence_error),
                        )
                    }
                }
            }
            ProviderTurnFailureCause::ExecutionBudget(canonical_error) => {
                match self.store.fail_turn_with_incident(
                    turn_id,
                    &error,
                    canonical_error.incident_id.as_deref(),
                ) {
                    Ok(turn) => (Some(turn), canonical_error),
                    Err(persistence_error) => {
                        eprintln!(
                            "santi: execution-budget turn persistence failed for {turn_id}: {persistence_error}"
                        );
                        (
                            None,
                            terminal_runtime_error(strand_id, turn_id, persistence_error),
                        )
                    }
                }
            }
            ProviderTurnFailureCause::Runtime(operation) => {
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
        metadata: ProviderFailureMetadata,
    ) -> (Option<Turn>, SantiError) {
        match self.store.fail_provider_turn(
            turn_id,
            error,
            ProviderFailureContext {
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
        operation: RuntimeTurnFailureOperation,
    ) -> (Option<Turn>, SantiError) {
        match self.store.fail_runtime_turn(
            turn_id,
            error,
            RuntimeFailureContext {
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
        match self.store.append_message(
            &turn.strand_id,
            ActorType::Soul,
            self.store.default_soul_id(),
            MessageContent::text(partial_assistant_text),
            MessageState::Aborted,
            MessageIntake::Record,
        ) {
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
    engine().transient(
        catalog::ERROR_ENGINE_PERSISTENCE_FAILED,
        ErrorSource::new("santi-core", "provider_turn_failure"),
        Some(ErrorScope::new("strand", strand_id)),
        "failed to persist provider failure incident",
        serde_json::json!({ "turn_id": turn_id, "detail": detail }),
    )
}

fn terminal_runtime_error(strand_id: &str, turn_id: &str, detail: String) -> SantiError {
    engine().transient(
        catalog::INTERNAL,
        ErrorSource::new("santi-core", "turn_failure_persistence"),
        Some(ErrorScope::new("strand", strand_id)),
        "failed to persist turn failure",
        serde_json::json!({ "turn_id": turn_id, "detail": detail }),
    )
}
