use crate::store::Ingress;
use crate::{Fault, engine, strand::Strand};

use super::super::{Service, drive};
use super::memory::{Gate, drive_maintenance};
use crate::{ingest, message, strand};

pub(in crate::service) struct Ingest<'a> {
    pub(in crate::service) content: message::Content,
    pub(in crate::service) kind: message::Kind,
    pub(in crate::service) trigger: &'a str,
    pub(in crate::service) source: Option<ingest::Source>,
    pub(in crate::service) replay: Option<crate::store::Replay<'a>>,
}

pub(in crate::service) struct External<'a> {
    pub(in crate::service) soul: &'a str,
    pub(in crate::service) label: &'a str,
    pub(in crate::service) text: String,
    pub(in crate::service) source: Option<ingest::Source>,
    pub(in crate::service) replay: Option<crate::store::Replay<'a>>,
}

struct Audit {
    kind: String,
    source: String,
    content_bytes: usize,
}

impl Audit {
    fn new(content: &message::Content, source: &Option<ingest::Source>) -> Self {
        Self {
            kind: source
                .as_ref()
                .map(|source| source.kind.as_str())
                .unwrap_or("unknown")
                .to_string(),
            source: source
                .as_ref()
                .and_then(|source| source.source.as_deref())
                .unwrap_or("-")
                .to_string(),
            content_bytes: content.rendered().len(),
        }
    }
}

#[derive(Clone, Copy)]
struct Drive<'a> {
    trigger: &'a str,
    accepted_inbox_id: Option<&'a str>,
    operation: &'a str,
    recover_failed_receipts: bool,
}

impl Service {
    pub fn ingest(
        &self,
        selector: strand::Selector,
        content: message::Content,
        kind: message::Kind,
        trigger: &str,
    ) -> Result<ingest::Outcome, String> {
        self.accept(
            selector,
            Ingest {
                content,
                kind,
                trigger,
                source: None,
                replay: None,
            },
        )
    }

    pub(in crate::service) fn accept(
        &self,
        selector: strand::Selector,
        input: Ingest<'_>,
    ) -> Result<ingest::Outcome, String> {
        let strand = self.store.resolve_strand_selector(&selector)?;
        let (outcome, _driven) = self.enqueue(&strand, input)?;
        Ok(outcome)
    }

    fn enqueue(
        &self,
        strand: &Strand,
        input: Ingest<'_>,
    ) -> Result<(ingest::Outcome, drive::Outcome), String> {
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
                if let ingest::Outcome::Rejected { error } = &outcome {
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
                ingest::Outcome::Rejected {
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
        if let ingest::Outcome::Rejected { error } = &outcome {
            log_ingest_rejection(error, &strand.id, &audit);
        }
        let drive = match &outcome {
            ingest::Outcome::Accepted { receipt } if intake.inserted => self.poke(
                &strand.id,
                input.trigger,
                Some(&receipt.inbox),
                "ingest_poke",
            ),
            ingest::Outcome::Accepted { .. } | ingest::Outcome::Rejected { .. } => {
                drive::Outcome::Idle
            }
        };
        let mut outcome = outcome;
        if let ingest::Outcome::Accepted { receipt } = &mut outcome {
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
        soul: &str,
        label: &str,
        system_text: String,
    ) -> Result<ingest::Outcome, String> {
        self.ingest_external_source(soul, label, system_text, None)
    }

    pub fn ingest_external_source(
        &self,
        soul: &str,
        label: &str,
        system_text: String,
        source: Option<ingest::Source>,
    ) -> Result<ingest::Outcome, String> {
        self.ingest_external(External {
            soul,
            label,
            text: system_text,
            source,
            replay: None,
        })
    }

    pub(in crate::service) fn ingest_external(
        &self,
        input: External<'_>,
    ) -> Result<ingest::Outcome, String> {
        let strand = self
            .store
            .resolve_strand_selector(&strand::Selector::ByLabel {
                soul: input.soul.to_string(),
                label: input.label.to_string(),
            })?;
        let (outcome, _driven) = self.enqueue(
            &strand,
            Ingest {
                content: message::Content::text(input.text),
                kind: message::Kind::SantiSystem,
                trigger: "system",
                source: input.source,
                replay: input.replay,
            },
        )?;
        Ok(outcome)
    }
}

mod dispatch;

fn log_ingest_rejection(error: &Fault, strand: &str, audit: &Audit) {
    eprintln!(
        "santi: ingest rejected code={} incident_id={} strand={} kind={} source={} content_bytes={}",
        error.code,
        error.incident.as_deref().unwrap_or("-"),
        strand,
        audit.kind,
        audit.source,
        audit.content_bytes,
    );
}

pub(super) fn send_error(
    descriptor: santi_error::Descriptor,
    strand: &str,
    message: String,
) -> Fault {
    engine().transient(crate::Signal {
        descriptor,
        source: santi_error::Source::new("santi-core", "strand_send"),
        scope: Some(santi_error::Scope::new("strand", strand)),
        message,
        context: serde_json::Value::Null,
    })
}
