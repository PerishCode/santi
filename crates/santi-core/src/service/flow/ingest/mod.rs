use crate::store::Ingress;
use crate::{Fault, engine, strand::Strand};

use super::super::{Service, drive};
use super::memory::{Gate, maintain};
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
    weight: usize,
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
            weight: content.rendered().len(),
        }
    }
}

#[derive(Clone, Copy)]
struct Drive<'a> {
    trigger: &'a str,
    inbox: Option<&'a str>,
    operation: &'a str,
    recovered: bool,
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
        let strand = self.store.selected(&selector)?;
        let (outcome, _driven) = self.enqueue(&strand, input)?;
        Ok(outcome)
    }

    fn enqueue(
        &self,
        strand: &Strand,
        input: Ingest<'_>,
    ) -> Result<(ingest::Outcome, drive::Outcome), String> {
        let audit = Audit::new(&input.content, &input.source);
        match self.gate(strand)? {
            Gate::Allow => {}
            Gate::Pause {
                maintenance_strand_id,
            } => {
                let intake = self.store.harbor(Ingress {
                    strand: &strand.id,
                    kind: input.kind,
                    content: input.content,
                    source: input.source,
                    admission: None,
                    replay: input.replay,
                })?;
                let outcome = intake.outcome;
                self.dispatched();
                if let ingest::Outcome::Rejected { error } = &outcome {
                    logged(error, &strand.id, &audit);
                }
                if intake.inserted {
                    maintain(self, &maintenance_strand_id);
                }
                return Ok((outcome, drive::Outcome::Paused));
            }
        }
        self.absolve(&strand.id, "ingest_remeasurement")?;
        if let Some(error) = self.store.gated(&strand.id)? {
            self.dispatched();
            logged(&error, &strand.id, &audit);
            return Ok((
                ingest::Outcome::Rejected {
                    error: Box::new(error),
                },
                drive::Outcome::Idle,
            ));
        }
        let admission = self.admission(&strand.id)?;
        let intake = self.store.ingest(Ingress {
            strand: &strand.id,
            kind: input.kind,
            content: input.content,
            source: input.source,
            admission: admission.as_ref(),
            replay: input.replay,
        })?;
        let outcome = intake.outcome;
        self.dispatched();
        if let ingest::Outcome::Rejected { error } = &outcome {
            logged(error, &strand.id, &audit);
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

    pub fn evented(
        &self,
        soul: &str,
        label: &str,
        system_text: String,
    ) -> Result<ingest::Outcome, String> {
        self.sourced(soul, label, system_text, None)
    }

    pub fn sourced(
        &self,
        soul: &str,
        label: &str,
        system_text: String,
        source: Option<ingest::Source>,
    ) -> Result<ingest::Outcome, String> {
        self.external(External {
            soul,
            label,
            text: system_text,
            source,
            replay: None,
        })
    }

    pub(in crate::service) fn external(
        &self,
        input: External<'_>,
    ) -> Result<ingest::Outcome, String> {
        let strand = self.store.selected(&strand::Selector::ByLabel {
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

fn logged(error: &Fault, strand: &str, audit: &Audit) {
    eprintln!(
        "santi: ingest rejected code={} incident_id={} strand={} kind={} source={} content_bytes={}",
        error.code,
        error.incident.as_deref().unwrap_or("-"),
        strand,
        audit.kind,
        audit.source,
        audit.weight,
    );
}

pub(super) fn erred(descriptor: santi_error::Descriptor, strand: &str, message: String) -> Fault {
    engine().transient(crate::Signal {
        descriptor,
        source: santi_error::Source::new("santi-core", "strand_send"),
        scope: Some(santi_error::Scope::new("strand", strand)),
        message,
        context: serde_json::Value::Null,
    })
}
