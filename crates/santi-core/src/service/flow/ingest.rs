use crate::store::{Ingress, Launch, Reservation, StartTurnOutcome, errors::drive::Input};
use crate::{
    DriveStrandResponse, DriveStrandState, ErrorScope, ErrorSource, InboxSource, IngestOutcome,
    MessageContent, MessageKind, SantiError, SantiStreamPayload, SendStrandAcceptedResponse,
    SendStrandRequest, Strand, StrandSelector, catalog, engine,
};

use super::super::{Service, drive};
use super::memory::{Gate, drive_maintenance};

pub(in crate::service) struct Ingest<'a> {
    pub(in crate::service) content: MessageContent,
    pub(in crate::service) kind: MessageKind,
    pub(in crate::service) trigger: &'a str,
    pub(in crate::service) source: Option<InboxSource>,
    pub(in crate::service) window: Option<Reservation<'a>>,
}

struct Audit {
    source_type: String,
    source_ref: String,
    content_bytes: usize,
}

impl Audit {
    fn new(content: &MessageContent, source: &Option<InboxSource>) -> Self {
        Self {
            source_type: source
                .as_ref()
                .map(|source| source.source_type.as_str())
                .unwrap_or("unknown")
                .to_string(),
            source_ref: source
                .as_ref()
                .and_then(|source| source.source_ref.as_deref())
                .unwrap_or("-")
                .to_string(),
            content_bytes: content.content_text().len(),
        }
    }
}

#[derive(Clone, Copy)]
struct Drive<'a> {
    trigger_type: &'a str,
    accepted_inbox_id: Option<&'a str>,
    operation: &'a str,
    recover_failed_receipts: bool,
}

impl Service {
    pub fn ingest(
        &self,
        selector: StrandSelector,
        content: MessageContent,
        kind: MessageKind,
        trigger_type: &str,
    ) -> Result<IngestOutcome, String> {
        self.accept(
            selector,
            Ingest {
                content,
                kind,
                trigger: trigger_type,
                source: None,
                window: None,
            },
        )
    }

    pub(in crate::service) fn accept(
        &self,
        selector: StrandSelector,
        input: Ingest<'_>,
    ) -> Result<IngestOutcome, String> {
        let strand = self.store.resolve_strand_selector(&selector)?;
        let (outcome, _driven) = self.enqueue(&strand, input)?;
        Ok(outcome)
    }

    fn enqueue(
        &self,
        strand: &Strand,
        input: Ingest<'_>,
    ) -> Result<(IngestOutcome, drive::Outcome), String> {
        let audit = Audit::new(&input.content, &input.source);
        match self.memory_drive_gate(strand)? {
            Gate::Allow => {}
            Gate::Pause {
                maintenance_strand_id,
            } => {
                let outcome = self.store.enqueue_inbox_while_suspended(
                    &strand.id,
                    input.kind,
                    input.content,
                    input.source,
                )?;
                self.dispatch_error_events();
                if let IngestOutcome::Rejected { error } = &outcome {
                    log_ingest_rejection(error, &strand.id, &audit);
                }
                drive_maintenance(self, &maintenance_strand_id);
                return Ok((outcome, drive::Outcome::Paused));
            }
        }
        self.clear_context_incident(&strand.id, "ingest_remeasurement")?;
        if let Some(error) = self.store.reject_if_drive_blocked(&strand.id)? {
            self.dispatch_error_events();
            log_ingest_rejection(&error, &strand.id, &audit);
            return Ok((
                IngestOutcome::Rejected {
                    error: Box::new(error),
                },
                drive::Outcome::Idle,
            ));
        }
        let admission = self.context_admission(&strand.id)?;
        let outcome = self.store.enqueue_inbox_with_context(Ingress {
            strand: &strand.id,
            kind: input.kind,
            content: input.content,
            source: input.source,
            admission: admission.as_ref(),
            window: input.window,
        })?;
        self.dispatch_error_events();
        if let IngestOutcome::Rejected { error } = &outcome {
            log_ingest_rejection(error, &strand.id, &audit);
        }
        let drive = match &outcome {
            IngestOutcome::Accepted { receipt } => self.poke(
                &strand.id,
                input.trigger,
                Some(&receipt.inbox_id),
                "ingest_poke",
            ),
            IngestOutcome::Rejected { .. } => drive::Outcome::Idle,
        };
        let mut outcome = outcome;
        if let IngestOutcome::Accepted { receipt } = &mut outcome {
            receipt.warning = match &drive {
                drive::Outcome::Failed(error) | drive::Outcome::Held(error) => {
                    Some(Box::new(error.clone()))
                }
                _ => None,
            };
        }
        Ok((outcome, drive))
    }

    pub fn ingest_external_event(
        &self,
        soul_id: &str,
        label: &str,
        system_text: String,
    ) -> Result<IngestOutcome, String> {
        self.ingest_external_source(soul_id, label, system_text, None)
    }

    pub fn ingest_external_source(
        &self,
        soul_id: &str,
        label: &str,
        system_text: String,
        source: Option<InboxSource>,
    ) -> Result<IngestOutcome, String> {
        let strand = self
            .store
            .resolve_strand_selector(&StrandSelector::ByLabel {
                soul_id: soul_id.to_string(),
                label: label.to_string(),
            })?;
        let (outcome, _driven) = self.enqueue(
            &strand,
            Ingest {
                content: MessageContent::text(system_text),
                kind: MessageKind::SantiSystem,
                trigger: "system",
                source,
                window: None,
            },
        )?;
        Ok(outcome)
    }
    pub async fn send_strand(
        &self,
        strand_id: &str,
        request: SendStrandRequest,
    ) -> Result<SendStrandAcceptedResponse, SantiError> {
        let text = request.text();
        if text.trim().is_empty() {
            return Err(send_error(
                catalog::INVALID_ARGUMENT,
                strand_id,
                "send content must contain text".to_string(),
            ));
        }
        let strand = self
            .store
            .strand(strand_id)
            .map_err(|message| send_error(catalog::INTERNAL, strand_id, message))?
            .ok_or_else(|| {
                send_error(
                    catalog::NOT_FOUND,
                    strand_id,
                    "strand not found".to_string(),
                )
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
                    window: None,
                },
            )
            .map_err(|message| send_error(catalog::INTERNAL, strand_id, message))?;
        let receipt = match outcome {
            IngestOutcome::Accepted { receipt } => receipt,
            IngestOutcome::Rejected { error } => return Err(*error),
        };
        let (turn, user_message) = match drive {
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
            user_message,
        })
    }

    pub fn drive_strand(&self, strand_id: &str) -> Result<DriveStrandResponse, Box<SantiError>> {
        self.store
            .strand(strand_id)
            .map_err(|message| Box::new(send_error(catalog::INTERNAL, strand_id, message)))?
            .ok_or_else(|| {
                Box::new(send_error(
                    catalog::NOT_FOUND,
                    strand_id,
                    "strand not found".to_string(),
                ))
            })?;
        match self.poke_failed_receipts(strand_id, "strand_send", None, "operator_redrive") {
            drive::Outcome::Started(turn, _) => Ok(DriveStrandResponse {
                strand_id: strand_id.to_string(),
                state: DriveStrandState::Started,
                turn: Some(turn),
            }),
            drive::Outcome::Running(turn) => Ok(DriveStrandResponse {
                strand_id: strand_id.to_string(),
                state: DriveStrandState::Running,
                turn: Some(turn),
            }),
            drive::Outcome::Idle => Ok(DriveStrandResponse {
                strand_id: strand_id.to_string(),
                state: DriveStrandState::Idle,
                turn: None,
            }),
            drive::Outcome::Paused => Ok(DriveStrandResponse {
                strand_id: strand_id.to_string(),
                state: DriveStrandState::Paused,
                turn: None,
            }),
            drive::Outcome::Held(error) | drive::Outcome::Failed(error) => Err(Box::new(error)),
        }
    }

    pub(in crate::service) fn poke(
        &self,
        strand_id: &str,
        trigger_type: &str,
        accepted_inbox_id: Option<&str>,
        operation: &str,
    ) -> drive::Outcome {
        self.poke_inner(
            strand_id,
            Drive {
                trigger_type,
                accepted_inbox_id,
                operation,
                recover_failed_receipts: false,
            },
        )
    }

    pub(in crate::service) fn poke_failed_receipts(
        &self,
        strand_id: &str,
        trigger_type: &str,
        accepted_inbox_id: Option<&str>,
        operation: &str,
    ) -> drive::Outcome {
        self.poke_inner(
            strand_id,
            Drive {
                trigger_type,
                accepted_inbox_id,
                operation,
                recover_failed_receipts: true,
            },
        )
    }

    fn poke_inner(&self, strand_id: &str, drive: Drive<'_>) -> drive::Outcome {
        if self.is_shutting_down() {
            return drive::Outcome::Paused;
        }
        let strand = match self.store.strand(strand_id) {
            Ok(Some(strand)) => strand,
            Ok(None) => {
                return drive::Outcome::Failed(self.record_drive_failure(
                    strand_id,
                    drive,
                    "strand not found".to_string(),
                ));
            }
            Err(error) => {
                return drive::Outcome::Failed(self.record_drive_failure(strand_id, drive, error));
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
                return drive::Outcome::Failed(self.record_drive_failure(strand_id, drive, error));
            }
        }
        if let Err(error) = self.clear_context_incident(strand_id, "driver_remeasurement") {
            return drive::Outcome::Failed(self.record_drive_failure(strand_id, drive, error));
        }
        let admission = match self.context_admission(strand_id) {
            Ok(admission) => admission,
            Err(error) => {
                return drive::Outcome::Failed(self.record_drive_failure(strand_id, drive, error));
            }
        };
        let started = self.store.start_turn_with_budget(Launch {
            strand: strand_id,
            trigger: drive.trigger_type,
            reference: None,
            admission: admission.as_ref(),
            recover: drive.recover_failed_receipts,
        });
        self.dispatch_error_events();
        match started {
            Ok(StartTurnOutcome::Started(started)) => {
                self.refresh_drive_health();
                for message in started.drained_messages.iter().cloned() {
                    self.publish_stream(strand_id, SantiStreamPayload::MessageCreated { message });
                }
                self.publish_stream(
                    strand_id,
                    SantiStreamPayload::TurnStarted {
                        turn: started.turn.clone(),
                    },
                );
                let background = self.clone();
                let background_strand_id = strand_id.to_string();
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
                drive::Outcome::Failed(self.record_drive_failure(strand_id, drive, error))
            }
        }
    }

    fn record_drive_failure(
        &self,
        strand_id: &str,
        drive: Drive<'_>,
        detail: String,
    ) -> SantiError {
        self.mark_drive_degraded();
        let error = match self.store.record_drive_failure(
            strand_id,
            Input {
                operation: drive.operation,
                trigger_type: drive.trigger_type,
                accepted_inbox_id: drive.accepted_inbox_id,
                detail: &detail,
            },
        ) {
            Ok(error) => error,
            Err(persistence_error) => engine().transient(crate::Signal {
                descriptor: catalog::ERROR_ENGINE_PERSISTENCE_FAILED,
                source: ErrorSource::new("santi-core", "strand_drive_failure"),
                scope: Some(ErrorScope::new("strand", strand_id)),
                message: "failed to persist strand driver incident".to_string(),
                context: serde_json::json!({
                    "accepted_before_failure": drive.accepted_inbox_id.is_some(),
                    "inbox_id": drive.accepted_inbox_id,
                    "detail": persistence_error,
                }),
            }),
        };
        eprintln!(
            "santi: strand drive failed code={} incident_id={} strand_id={} operation={} accepted_before_failure={}",
            error.code,
            error.incident_id.as_deref().unwrap_or("-"),
            strand_id,
            drive.operation,
            drive.accepted_inbox_id.is_some(),
        );
        self.dispatch_error_events();
        error
    }
}

fn log_ingest_rejection(error: &SantiError, strand_id: &str, audit: &Audit) {
    eprintln!(
        "santi: ingest rejected code={} incident_id={} strand_id={} source_type={} source_ref={} content_bytes={}",
        error.code,
        error.incident_id.as_deref().unwrap_or("-"),
        strand_id,
        audit.source_type,
        audit.source_ref,
        audit.content_bytes,
    );
}

fn send_error(
    descriptor: santi_error::ErrorDescriptor,
    strand_id: &str,
    message: String,
) -> SantiError {
    engine().transient(crate::Signal {
        descriptor,
        source: ErrorSource::new("santi-core", "strand_send"),
        scope: Some(ErrorScope::new("strand", strand_id)),
        message,
        context: serde_json::Value::Null,
    })
}
