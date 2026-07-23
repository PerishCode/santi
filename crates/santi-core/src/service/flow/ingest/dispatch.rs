use crate::service::flow::memory::{Gate, drive_maintenance};
use crate::service::{Service, drive};
use crate::store::{Launch, StartTurnOutcome, errors::drive::Input};
use crate::{
    DriveStrandResponse, DriveStrandState, Fault, InboxSource, IngestOutcome, MessageContent,
    MessageKind, SantiStreamPayload, SendStrandAcceptedResponse, SendStrandRequest, catalog,
    engine,
};

use super::*;

impl Service {
    pub async fn send_strand(
        &self,
        strand: &str,
        request: SendStrandRequest,
    ) -> Result<SendStrandAcceptedResponse, Fault> {
        let text = request.text();
        if text.trim().is_empty() {
            return Err(send_error(
                catalog::INVALID_ARGUMENT,
                strand,
                "send content must contain text".to_string(),
            ));
        }
        let strand = self
            .store
            .strand(strand)
            .map_err(|message| send_error(catalog::INTERNAL, strand, message))?
            .ok_or_else(|| {
                send_error(catalog::NOT_FOUND, strand, "strand not found".to_string())
            })?;

        let (outcome, drive) = self
            .enqueue(
                &strand,
                Ingest {
                    content: MessageContent {
                        parts: request.content,
                    },
                    kind: MessageKind::Text,
                    trigger: "strand_send",
                    source: Some(InboxSource::new("strand_send").with_ref(strand.id.clone())),
                    replay: None,
                },
            )
            .map_err(|message| send_error(catalog::INTERNAL, &strand.id, message))?;
        let receipt = match outcome {
            IngestOutcome::Accepted { receipt } => receipt,
            IngestOutcome::Rejected { error } => return Err(*error),
        };
        let (turn, message) = match drive {
            drive::Outcome::Started(turn, mut drained) => (Some(turn), drained.pop()),
            drive::Outcome::Running(turn) => (Some(turn), None),
            drive::Outcome::Idle
            | drive::Outcome::Held(_)
            | drive::Outcome::Paused
            | drive::Outcome::Failed(_) => (None, None),
        };

        Ok(SendStrandAcceptedResponse {
            strand,
            receipt,
            turn,
            message,
        })
    }

    pub fn drive_strand(&self, strand: &str) -> Result<DriveStrandResponse, Box<Fault>> {
        self.store
            .strand(strand)
            .map_err(|message| Box::new(send_error(catalog::INTERNAL, strand, message)))?
            .ok_or_else(|| {
                Box::new(send_error(
                    catalog::NOT_FOUND,
                    strand,
                    "strand not found".to_string(),
                ))
            })?;
        match self.poke_failed_receipts(strand, "strand_send", None, "operator_redrive") {
            drive::Outcome::Started(turn, _) => Ok(DriveStrandResponse {
                strand: strand.to_string(),
                state: DriveStrandState::Started,
                turn: Some(turn),
            }),
            drive::Outcome::Running(turn) => Ok(DriveStrandResponse {
                strand: strand.to_string(),
                state: DriveStrandState::Running,
                turn: Some(turn),
            }),
            drive::Outcome::Idle => Ok(DriveStrandResponse {
                strand: strand.to_string(),
                state: DriveStrandState::Idle,
                turn: None,
            }),
            drive::Outcome::Paused => Ok(DriveStrandResponse {
                strand: strand.to_string(),
                state: DriveStrandState::Paused,
                turn: None,
            }),
            drive::Outcome::Held(error) | drive::Outcome::Failed(error) => Err(Box::new(error)),
        }
    }

    pub(in crate::service) fn poke(
        &self,
        strand: &str,
        trigger: &str,
        accepted_inbox_id: Option<&str>,
        operation: &str,
    ) -> drive::Outcome {
        self.poke_inner(
            strand,
            Drive {
                trigger,
                accepted_inbox_id,
                operation,
                recover_failed_receipts: false,
            },
        )
    }

    pub(in crate::service) fn poke_failed_receipts(
        &self,
        strand: &str,
        trigger: &str,
        accepted_inbox_id: Option<&str>,
        operation: &str,
    ) -> drive::Outcome {
        self.poke_inner(
            strand,
            Drive {
                trigger,
                accepted_inbox_id,
                operation,
                recover_failed_receipts: true,
            },
        )
    }

    fn poke_inner(&self, strand: &str, drive: Drive<'_>) -> drive::Outcome {
        if self.is_shutting_down() {
            return drive::Outcome::Paused;
        }
        let strand = match self.store.strand(strand) {
            Ok(Some(strand)) => strand,
            Ok(None) => {
                return drive::Outcome::Failed(self.record_drive_failure(
                    strand,
                    drive,
                    "strand not found".to_string(),
                ));
            }
            Err(error) => {
                return drive::Outcome::Failed(self.record_drive_failure(strand, drive, error));
            }
        };
        match self.memory_drive_gate(&strand) {
            Ok(Gate::Allow) => {}
            Ok(Gate::Pause {
                maintenance_strand_id,
            }) => {
                drive_maintenance(self, &maintenance_strand_id);
                return drive::Outcome::Paused;
            }
            Err(error) => {
                return drive::Outcome::Failed(self.record_drive_failure(&strand.id, drive, error));
            }
        }
        if let Err(error) = self.clear_context_incident(&strand.id, "driver_remeasurement") {
            return drive::Outcome::Failed(self.record_drive_failure(&strand.id, drive, error));
        }
        let admission = match self.context_admission(&strand.id) {
            Ok(admission) => admission,
            Err(error) => {
                return drive::Outcome::Failed(self.record_drive_failure(&strand.id, drive, error));
            }
        };
        let started = self.store.start_turn_with_budget(Launch {
            strand: &strand.id,
            trigger: drive.trigger,
            reference: None,
            admission: admission.as_ref(),
            recover: drive.recover_failed_receipts,
        });
        self.dispatch_error_events();
        match started {
            Ok(StartTurnOutcome::Started(started)) => {
                self.refresh_drive_health();
                for message in started.drained_messages.iter().cloned() {
                    self.publish_stream(&strand.id, SantiStreamPayload::MessageCreated { message });
                }
                self.publish_stream(
                    &strand.id,
                    SantiStreamPayload::TurnStarted {
                        turn: started.turn.clone(),
                    },
                );
                let background = self.clone();
                let background_strand_id = strand.id.clone();
                let background_turn_id = started.turn.id.clone();
                tokio::spawn(async move {
                    background
                        .complete_provider_turn(background_strand_id, background_turn_id)
                        .await;
                });
                drive::Outcome::Started(started.turn, started.drained_messages)
            }
            Ok(StartTurnOutcome::Running(turn)) => drive::Outcome::Running(turn),
            Ok(StartTurnOutcome::Idle) => drive::Outcome::Idle,
            Ok(StartTurnOutcome::Held(error)) => drive::Outcome::Held(error),
            Err(error) => {
                drive::Outcome::Failed(self.record_drive_failure(&strand.id, drive, error))
            }
        }
    }

    fn record_drive_failure(&self, strand: &str, drive: Drive<'_>, detail: String) -> Fault {
        self.mark_drive_degraded();
        let error = match self.store.record_drive_failure(
            strand,
            Input {
                operation: drive.operation,
                trigger: drive.trigger,
                accepted_inbox_id: drive.accepted_inbox_id,
                detail: &detail,
            },
        ) {
            Ok(error) => error,
            Err(persistence_error) => engine().transient(crate::Signal {
                descriptor: catalog::ERROR_ENGINE_PERSISTENCE_FAILED,
                source: santi_error::Source::new("santi-core", "strand_drive_failure"),
                scope: Some(santi_error::Scope::new("strand", strand)),
                message: "failed to persist strand driver incident".to_string(),
                context: serde_json::json!({
                    "accepted_before_failure": drive.accepted_inbox_id.is_some(),
                    "inbox": drive.accepted_inbox_id,
                    "detail": persistence_error,
                }),
            }),
        };
        eprintln!(
            "santi: strand drive failed code={} incident_id={} strand={} operation={} accepted_before_failure={}",
            error.code,
            error.incident.as_deref().unwrap_or("-"),
            strand,
            drive.operation,
            drive.accepted_inbox_id.is_some(),
        );
        self.dispatch_error_events();
        error
    }
}
