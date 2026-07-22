use crate::store::Ingress;
use crate::{
    ErrorScope, ErrorSource, InboxSource, IngestOutcome, MessageContent, MessageKind, SantiError,
    Strand, StrandSelector, engine,
};

use super::super::{Service, drive};
use super::memory::{Gate, drive_maintenance};

pub(in crate::service) struct Ingest<'a> {
    pub(in crate::service) content: MessageContent,
    pub(in crate::service) kind: MessageKind,
    pub(in crate::service) trigger: &'a str,
    pub(in crate::service) source: Option<InboxSource>,
    pub(in crate::service) replay: Option<crate::store::Replay<'a>>,
}

pub(in crate::service) struct External<'a> {
    pub(in crate::service) soul: &'a str,
    pub(in crate::service) label: &'a str,
    pub(in crate::service) text: String,
    pub(in crate::service) source: Option<InboxSource>,
    pub(in crate::service) replay: Option<crate::store::Replay<'a>>,
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
                replay: None,
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
                let intake = self.store.enqueue_inbox_while_suspended(Ingress {
                    strand: &strand.id,
                    kind: input.kind,
                    content: input.content,
                    source: input.source,
                    admission: None,
                    replay: input.replay,
                })?;
                let outcome = intake.outcome;
                self.dispatch_error_events();
                if let IngestOutcome::Rejected { error } = &outcome {
                    log_ingest_rejection(error, &strand.id, &audit);
                }
                if intake.inserted {
                    drive_maintenance(self, &maintenance_strand_id);
                }
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
        let intake = self.store.enqueue_inbox_with_context(Ingress {
            strand: &strand.id,
            kind: input.kind,
            content: input.content,
            source: input.source,
            admission: admission.as_ref(),
            replay: input.replay,
        })?;
        let outcome = intake.outcome;
        self.dispatch_error_events();
        if let IngestOutcome::Rejected { error } = &outcome {
            log_ingest_rejection(error, &strand.id, &audit);
        }
        let drive = match &outcome {
            IngestOutcome::Accepted { receipt } if intake.inserted => self.poke(
                &strand.id,
                input.trigger,
                Some(&receipt.inbox_id),
                "ingest_poke",
            ),
            IngestOutcome::Accepted { .. } | IngestOutcome::Rejected { .. } => drive::Outcome::Idle,
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
        self.ingest_external(External {
            soul: soul_id,
            label,
            text: system_text,
            source,
            replay: None,
        })
    }

    pub(in crate::service) fn ingest_external(
        &self,
        input: External<'_>,
    ) -> Result<IngestOutcome, String> {
        let strand = self
            .store
            .resolve_strand_selector(&StrandSelector::ByLabel {
                soul_id: input.soul.to_string(),
                label: input.label.to_string(),
            })?;
        let (outcome, _driven) = self.enqueue(
            &strand,
            Ingest {
                content: MessageContent::text(input.text),
                kind: MessageKind::SantiSystem,
                trigger: "system",
                source: input.source,
                replay: input.replay,
            },
        )?;
        Ok(outcome)
    }
}

mod dispatch;

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

pub(super) fn send_error(
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
