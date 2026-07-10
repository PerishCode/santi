use crate::store::{DriveFailureInput, StartTurnOutcome};
use crate::{
    DriveStrandResponse, DriveStrandState, ErrorScope, ErrorSource, InboxSource, IngestOutcome,
    MessageContent, MessageKind, SantiError, SantiStreamPayload, SendStrandAcceptedResponse,
    SendStrandRequest, Strand, StrandSelector, catalog, engine,
};

use super::super::{DriveOutcome, SantiService};

impl SantiService {
    /// The one inbound path (see PHASE-06/STEP4): resolve `selector` to a
    /// strand, enqueue `content` into its durable inbox, try to drive a turn.
    /// `Accepted` only confirms durable enqueue, not that a message/turn now
    /// exists yet — the driver may still be draining a running turn's inbox
    /// later (see `ingest_into`). `Rejected` (the inbox gate — a scale safety
    /// valve, not an error) is a normal outcome; handling it is the adaptor's
    /// own policy (surface it, or silently drop + log).
    pub fn ingest(
        &self,
        selector: StrandSelector,
        content: MessageContent,
        kind: MessageKind,
        trigger_type: &str,
    ) -> Result<IngestOutcome, String> {
        self.ingest_with_source(selector, content, kind, trigger_type, None)
    }

    pub fn ingest_with_source(
        &self,
        selector: StrandSelector,
        content: MessageContent,
        kind: MessageKind,
        trigger_type: &str,
        source: Option<InboxSource>,
    ) -> Result<IngestOutcome, String> {
        let strand = self.store.resolve_strand_selector(&selector)?;
        let (outcome, _driven) = self.ingest_into(&strand, content, kind, trigger_type, source)?;
        Ok(outcome)
    }

    /// Shared ingest core (enqueue + drive) for both the generic `ingest` and
    /// `send_strand` (which additionally wants the turn/message it may have
    /// just driven, to shape its richer response). Returns `driven = Some` only
    /// when THIS call's poke actually drained the inbox (a fresh turn started,
    /// possibly covering other adaptors' concurrently-enqueued entries too) —
    /// `None` when it coalesced into an already-running turn, whose own
    /// completion re-check will drain this content later.
    fn ingest_into(
        &self,
        strand: &Strand,
        content: MessageContent,
        kind: MessageKind,
        trigger_type: &str,
        source: Option<InboxSource>,
    ) -> Result<(IngestOutcome, DriveOutcome), String> {
        let audit_source_type = source
            .as_ref()
            .map(|source| source.source_type.as_str())
            .unwrap_or("unknown")
            .to_string();
        let audit_source_ref = source
            .as_ref()
            .and_then(|source| source.source_ref.as_deref())
            .unwrap_or("-")
            .to_string();
        let audit_content_bytes = content.content_text().len();
        if let Some(error) = self.store.reject_if_drive_blocked(&strand.id)? {
            self.dispatch_error_events();
            log_ingest_rejection(
                &error,
                &strand.id,
                &audit_source_type,
                &audit_source_ref,
                audit_content_bytes,
            );
            return Ok((
                IngestOutcome::Rejected {
                    error: Box::new(error),
                },
                DriveOutcome::Idle,
            ));
        }
        let admission = self.context_admission(&strand.id)?;
        let outcome = self.store.enqueue_inbox_with_context(
            &strand.id,
            kind,
            content,
            source,
            admission.as_ref(),
        )?;
        self.dispatch_error_events();
        if let IngestOutcome::Rejected { error } = &outcome {
            log_ingest_rejection(
                error,
                &strand.id,
                &audit_source_type,
                &audit_source_ref,
                audit_content_bytes,
            );
        }
        let drive = match &outcome {
            IngestOutcome::Accepted { receipt } => self.poke(
                &strand.id,
                trigger_type,
                Some(&receipt.inbox_id),
                "ingest_poke",
            ),
            IngestOutcome::Rejected { .. } => DriveOutcome::Idle,
        };
        let mut outcome = outcome;
        if let IngestOutcome::Accepted { receipt } = &mut outcome {
            receipt.warning = match &drive {
                DriveOutcome::Failed(error) | DriveOutcome::Held(error) => {
                    Some(Box::new(error.clone()))
                }
                _ => None,
            };
        }
        Ok((outcome, drive))
    }

    /// Ingest an external event already normalized by an adaptor: a `santi-system`
    /// message addressed to `soul_id`, anchored to the strand bound to `label`.
    /// This is the webhook twin of `send_strand` — same `ingest_into` core, so
    /// the same drive/coalesce/gate semantics. Core stays generic: the label and
    /// the message text are opaque (the adaptor owns their meaning).
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
        let (outcome, _driven) = self.ingest_into(
            &strand,
            MessageContent::text(system_text),
            MessageKind::SantiSystem,
            "system",
            source,
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

        // ingest_into is decoupled from drive: enqueue into the inbox, then try
        // to drive a turn. If one is already running for this strand, this send
        // simply joins the thread (coalesced) and the running turn (or its
        // completion re-check) will drain it later — no second concurrent turn.
        let (outcome, drive) = self
            .ingest_into(
                &strand,
                MessageContent {
                    parts: request.content,
                },
                MessageKind::Text,
                "strand_send",
                Some(InboxSource::new("strand_send").with_ref(strand.id.clone())),
            )
            .map_err(|message| send_error(catalog::INTERNAL, strand_id, message))?;
        let receipt = match outcome {
            IngestOutcome::Accepted { receipt } => receipt,
            IngestOutcome::Rejected { error } => return Err(*error),
        };
        let (turn, user_message) = match drive {
            DriveOutcome::Started(turn, mut drained) => (Some(turn), drained.pop()),
            DriveOutcome::Running(turn) => (Some(turn), None),
            DriveOutcome::Idle
            | DriveOutcome::Held(_)
            | DriveOutcome::Paused
            | DriveOutcome::Failed(_) => (None, None),
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
        match self.poke(strand_id, "strand_send", None, "operator_redrive") {
            DriveOutcome::Started(turn, _) => Ok(DriveStrandResponse {
                strand_id: strand_id.to_string(),
                state: DriveStrandState::Started,
                turn: Some(turn),
            }),
            DriveOutcome::Running(turn) => Ok(DriveStrandResponse {
                strand_id: strand_id.to_string(),
                state: DriveStrandState::Running,
                turn: Some(turn),
            }),
            DriveOutcome::Idle => Ok(DriveStrandResponse {
                strand_id: strand_id.to_string(),
                state: DriveStrandState::Idle,
                turn: None,
            }),
            DriveOutcome::Paused => Ok(DriveStrandResponse {
                strand_id: strand_id.to_string(),
                state: DriveStrandState::Paused,
                turn: None,
            }),
            DriveOutcome::Held(error) | DriveOutcome::Failed(error) => Err(Box::new(error)),
        }
    }

    /// Drive a turn if the strand is behind and idle. The outcome distinguishes
    /// normal coalescing/idle/paused states from context holds and driver
    /// failures so each transport can surface the accepted-write truth.
    pub(in crate::service) fn poke(
        &self,
        strand_id: &str,
        trigger_type: &str,
        accepted_inbox_id: Option<&str>,
        operation: &str,
    ) -> DriveOutcome {
        // Graceful shutdown: pause CONSUMPTION. The content stays durably in the
        // inbox (ingest already enqueued it) and boot recovery drains it on the
        // next start. This also stops the completion re-poke from spawning a
        // follow-on turn, so an in-flight turn can finish and the strand quiesce.
        if self.is_shutting_down() {
            return DriveOutcome::Paused;
        }
        let admission = match self.context_admission(strand_id) {
            Ok(admission) => admission,
            Err(error) => {
                return DriveOutcome::Failed(self.record_drive_failure(
                    strand_id,
                    trigger_type,
                    accepted_inbox_id,
                    operation,
                    error,
                ));
            }
        };
        let started =
            self.store
                .start_turn_with_budget(strand_id, trigger_type, None, admission.as_ref());
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
                DriveOutcome::Started(started.turn, started.drained_messages)
            }
            Ok(StartTurnOutcome::Running(turn)) => DriveOutcome::Running(turn),
            Ok(StartTurnOutcome::Idle) => DriveOutcome::Idle,
            Ok(StartTurnOutcome::Held(error)) => DriveOutcome::Held(error),
            Err(error) => DriveOutcome::Failed(self.record_drive_failure(
                strand_id,
                trigger_type,
                accepted_inbox_id,
                operation,
                error,
            )),
        }
    }

    fn record_drive_failure(
        &self,
        strand_id: &str,
        trigger_type: &str,
        accepted_inbox_id: Option<&str>,
        operation: &str,
        detail: String,
    ) -> SantiError {
        self.mark_drive_degraded();
        let error = match self.store.record_drive_failure(
            strand_id,
            DriveFailureInput {
                operation,
                trigger_type,
                accepted_inbox_id,
                detail: &detail,
            },
        ) {
            Ok(error) => error,
            Err(persistence_error) => engine().transient(
                catalog::ERROR_ENGINE_PERSISTENCE_FAILED,
                ErrorSource::new("santi-core", "strand_drive_failure"),
                Some(ErrorScope::new("strand", strand_id)),
                "failed to persist strand driver incident",
                serde_json::json!({
                    "accepted_before_failure": accepted_inbox_id.is_some(),
                    "inbox_id": accepted_inbox_id,
                    "detail": persistence_error,
                }),
            ),
        };
        eprintln!(
            "santi: strand drive failed code={} incident_id={} strand_id={} operation={} accepted_before_failure={}",
            error.code,
            error.incident_id.as_deref().unwrap_or("-"),
            strand_id,
            operation,
            accepted_inbox_id.is_some(),
        );
        self.dispatch_error_events();
        error
    }
}

fn log_ingest_rejection(
    error: &SantiError,
    strand_id: &str,
    source_type: &str,
    source_ref: &str,
    content_bytes: usize,
) {
    eprintln!(
        "santi: ingest rejected code={} incident_id={} strand_id={} source_type={} source_ref={} content_bytes={}",
        error.code,
        error.incident_id.as_deref().unwrap_or("-"),
        strand_id,
        source_type,
        source_ref,
        content_bytes,
    );
}

fn send_error(
    descriptor: santi_error::ErrorDescriptor,
    strand_id: &str,
    message: String,
) -> SantiError {
    engine().transient(
        descriptor,
        ErrorSource::new("santi-core", "strand_send"),
        Some(ErrorScope::new("strand", strand_id)),
        message,
        serde_json::Value::Null,
    )
}
