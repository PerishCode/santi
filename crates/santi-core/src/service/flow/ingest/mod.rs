use crate::{Fault, engine, strand::Strand};

use super::super::{Service, drive};
use super::memory::{Gate, maintain};
use crate::{ingest, message, strand};

pub(in crate::service) struct Ingest<'a> {
    pub(in crate::service) content: message::Content,
    pub(in crate::service) kind: message::Kind,
    pub(in crate::service) trigger: &'a str,
    pub(in crate::service) source: Option<ingest::Source>,
    pub(in crate::service) replay: Option<santi_estate::ReplayDraft<'a>>,
}

pub(in crate::service) struct External<'a> {
    pub(in crate::service) soul: &'a str,
    pub(in crate::service) label: &'a str,
    pub(in crate::service) text: String,
    pub(in crate::service) source: Option<ingest::Source>,
    pub(in crate::service) replay: Option<santi_estate::ReplayDraft<'a>>,
}

struct Intake {
    outcome: ingest::Outcome,
    inserted: bool,
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
}

impl Service {
    pub(in crate::service) async fn notify(
        &self,
        attention: santi_estate::AttentionDraft<'_>,
        notice: santi_estate::NoticeDraft<'_>,
    ) -> Result<(), String> {
        let strand = notice.strand.to_string();
        let offered = self.store.attend_job(attention, notice, 500).await?;
        self.dispatched().await;
        if offered.inserted
            && let Some(inbox) = offered.inbox
        {
            self.inboxes.lock().unwrap().insert(strand, inbox);
        }
        Ok(())
    }

    pub(in crate::service) async fn rouse(&self) {
        let pending = std::mem::take(&mut *self.inboxes.lock().unwrap());
        for (strand, inbox) in pending {
            let outcome = self
                .poke(&strand, "system", Some(&inbox), "inbox_notice_poke")
                .await;
            if let drive::Outcome::Failed(error) = outcome {
                eprintln!(
                    "santi: inbox notice wake failed strand={} code={} detail={}",
                    strand, error.code, error.message
                );
            }
        }
    }

    pub async fn ingest(
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
        .await
    }

    pub(in crate::service) async fn accept(
        &self,
        selector: strand::Selector,
        input: Ingest<'_>,
    ) -> Result<ingest::Outcome, String> {
        let strand = self.store.selected(&selector, &crate::now()).await?;
        let (outcome, _driven) = self.enqueue(&strand, input).await?;
        Ok(outcome)
    }

    async fn enqueue(
        &self,
        strand: &Strand,
        input: Ingest<'_>,
    ) -> Result<(ingest::Outcome, drive::Outcome), String> {
        let audit = Audit::new(&input.content, &input.source);
        match self.gate(strand).await? {
            Gate::Allow => {}
            Gate::Pause {
                maintenance_strand_id,
            } => {
                let intake = self
                    .intake(
                        &strand.id,
                        input.kind,
                        input.content,
                        input.source,
                        input.replay,
                    )
                    .await?;
                let outcome = intake.outcome;
                self.dispatched().await;
                if let ingest::Outcome::Rejected { error } = &outcome {
                    logged(error, &strand.id, &audit);
                }
                if intake.inserted {
                    maintain(self, &maintenance_strand_id).await;
                }
                return Ok((outcome, drive::Outcome::Paused));
            }
        }
        self.absolve(&strand.id, "ingest_remeasurement").await?;
        if let Some(error) = self.gated(&strand.id, "ingest_active_guard").await? {
            self.dispatched().await;
            logged(&error, &strand.id, &audit);
            return Ok((
                ingest::Outcome::Rejected {
                    error: Box::new(error),
                },
                drive::Outcome::Idle,
            ));
        }
        if let Some(error) = self
            .admit_candidate(&strand.id, &input.kind, &input.content)
            .await?
        {
            self.dispatched().await;
            logged(&error, &strand.id, &audit);
            return Ok((
                ingest::Outcome::Rejected {
                    error: Box::new(error),
                },
                drive::Outcome::Idle,
            ));
        }
        let intake = self
            .intake(
                &strand.id,
                input.kind,
                input.content,
                input.source,
                input.replay,
            )
            .await?;
        let outcome = intake.outcome;
        self.dispatched().await;
        if let ingest::Outcome::Rejected { error } = &outcome {
            logged(error, &strand.id, &audit);
        }
        let drive = match &outcome {
            ingest::Outcome::Accepted { receipt } if intake.inserted => {
                self.poke(
                    &strand.id,
                    input.trigger,
                    Some(&receipt.inbox),
                    "ingest_poke",
                )
                .await
            }
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

    async fn intake(
        &self,
        strand: &str,
        kind: message::Kind,
        content: message::Content,
        source: Option<ingest::Source>,
        replay: Option<santi_estate::ReplayDraft<'_>>,
    ) -> Result<Intake, String> {
        let inbox = crate::tag("inbox");
        let created = crate::now();
        let draft = santi_estate::InboxDraft {
            tag: &inbox,
            strand,
            kind,
            content: &content,
            source: source.as_ref(),
            created: &created,
        };
        match replay {
            Some(replay) => {
                let accepted = self.store.accept_replay(draft, replay, 500).await?;
                Ok(Intake {
                    outcome: ingest::Outcome::Accepted {
                        receipt: accepted.receipt,
                    },
                    inserted: accepted.inserted,
                })
            }
            None => {
                self.store.accept_inbox(draft, 500).await?;
                Ok(Intake {
                    outcome: ingest::Outcome::Accepted {
                        receipt: ingest::Receipt {
                            strand: strand.to_string(),
                            inbox,
                            warning: None,
                        },
                    },
                    inserted: true,
                })
            }
        }
    }
}

mod dispatch;
mod external;
mod face;
mod failure;

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
