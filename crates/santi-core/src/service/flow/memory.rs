use crate::Ruled;
use std::fs;

use santi_provider::Item;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::store::Ingress;
use crate::{soulward, strand::Strand};

use super::super::{Service, drive};
use crate::{ingest, message};

const FALLBACK: usize = 500_000;
pub(super) const MAINTENANCE: &str = "santi:memory:maintenance";

#[derive(Clone, Copy)]
pub(in crate::service) struct Policy {
    pub(in crate::service) allowance: usize,
    threshold: usize,
}

pub(super) enum Gate {
    Allow,
    Pause { maintenance_strand_id: String },
}

struct Snapshot {
    weight: usize,
    sha256: String,
}

impl Service {
    pub(in crate::service) fn regimen(&self) -> Policy {
        let bytes = self
            .provider
            .metadata()
            .budget
            .map_or(FALLBACK, |budget| budget.bytes);
        let allowance = (bytes / 2).max(1);
        let threshold = (bytes.saturating_mul(3) / 4).max(allowance.saturating_add(1));
        Policy {
            allowance,
            threshold,
        }
    }

    pub(super) fn gate(&self, strand: &Strand) -> Result<Gate, String> {
        let _guard = self.pressure.lock().unwrap();
        let policy = self.regimen();
        let snapshot = self.measure(&strand.soul)?;
        self.reconcile(&strand.soul, &snapshot, policy)?;
        if snapshot.weight <= policy.allowance {
            return Ok(Gate::Allow);
        }

        let maintenance = self.store.labeled(&strand.soul, MAINTENANCE)?;
        self.brief(&maintenance, &snapshot, policy)?;
        if strand.id == maintenance.id {
            Ok(Gate::Allow)
        } else {
            Ok(Gate::Pause {
                maintenance_strand_id: maintenance.id,
            })
        }
    }

    pub(super) fn relieve(&self, strand: &str) {
        if let Err(error) = self.remeasure(strand) {
            eprintln!("santi: soul memory relief scan failed strand={strand}: {error}");
        }
    }

    fn remeasure(&self, strand: &str) -> Result<(), String> {
        let Some(maintenance) = self.store.strand(strand)? else {
            return Ok(());
        };
        if maintenance.label.as_deref() != Some(MAINTENANCE) {
            return Ok(());
        }
        let policy = self.regimen();
        let snapshot = self.measure(&maintenance.soul)?;
        self.reconcile(&maintenance.soul, &snapshot, policy)?;
        if snapshot.weight > policy.allowance {
            return Ok(());
        }

        for pending_id in self.store.awaiting()? {
            if pending_id == strand {
                continue;
            }
            let Some(pending) = self.store.strand(&pending_id)? else {
                continue;
            };
            if pending.soul == maintenance.soul {
                let _ = self.poke(
                    &pending_id,
                    "strand_send",
                    None,
                    "soul_memory_relief_resume",
                );
            }
        }
        Ok(())
    }

    fn measure(&self, soul: &str) -> Result<Snapshot, String> {
        let path = self.memoir(soul);
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error.to_string()),
        };
        Ok(Snapshot {
            weight: bytes.len(),
            sha256: hex::encode(Sha256::digest(&bytes)),
        })
    }

    fn brief(
        &self,
        maintenance: &Strand,
        snapshot: &Snapshot,
        policy: Policy,
    ) -> Result<(), String> {
        let fingerprint = format!("source_sha256: {}", snapshot.sha256);
        let recorded = self
            .store
            .messages(&maintenance.id)?
            .iter()
            .any(|message| message.text.contains(&fingerprint));
        let pending = self
            .store
            .pending(&maintenance.id)?
            .iter()
            .any(|item| contains(item, &fingerprint));
        if recorded || pending {
            return Ok(());
        }

        let outcome = self.store.harbor(Ingress {
            strand: &maintenance.id,
            kind: message::Kind::SantiSystem,
            content: metaprompt(snapshot, policy),
            source: Some(
                ingest::Source::new("runtime_memory_pressure").with_ref(maintenance.soul.clone()),
            ),
            admission: None,
            replay: None,
        })?;
        match outcome.outcome {
            ingest::Outcome::Accepted { .. } => Ok(()),
            ingest::Outcome::Rejected { error } => Err(format!(
                "memory maintenance metaprompt was rejected: {}",
                error.message
            )),
        }
    }

    fn reconcile(&self, soul: &str, snapshot: &Snapshot, policy: Policy) -> Result<(), String> {
        let key = crate::soul::Error::Intervention
            .descriptor()
            .key("soul", soul);
        let active = self.store.incident(&key)?;
        let mutated = if snapshot.weight > policy.threshold {
            if active.is_some() {
                false
            } else {
                self.store.raise(santi_error::Draft {
                    key,
                    descriptor: crate::soul::Error::Intervention.descriptor(),
                    scope: santi_error::Scope::new("soul", soul),
                    source: santi_error::Source::new("santi-core", "soul_memory_pressure"),
                    message: "soul memory exceeds the human-intervention threshold".to_string(),
                    context: json!({
                        "schema": "santi.error.soul_memory.v1",
                        "source": soulward(),
                        "source_bytes": snapshot.weight,
                        "allowance_bytes": policy.allowance,
                        "operator_threshold_bytes": policy.threshold,
                        "maintenance_label": MAINTENANCE,
                        "runtime_mutated_memory": false,
                    }),
                })?;
                true
            }
        } else if active.is_some() {
            self.store.resolve(
                &key,
                "soul_memory_remeasured",
                json!({
                    "schema": "santi.error.soul_memory.resolution.v1",
                    "source_bytes": snapshot.weight,
                    "allowance_bytes": policy.allowance,
                    "operator_threshold_bytes": policy.threshold,
                }),
            )?
        } else {
            false
        };
        if mutated {
            self.dispatched();
        }
        Ok(())
    }
}

fn metaprompt(snapshot: &Snapshot, policy: Policy) -> message::Content {
    message::Content::text(
        [
            "<system_message>".to_string(),
            "kind: soul_memory_maintenance".to_string(),
            "scope: soul".to_string(),
            "wake: true".to_string(),
            format!("source: {}", soulward()),
            format!("source_sha256: {}", snapshot.sha256),
            format!("source_bytes: {}", snapshot.weight),
            format!("allowance_bytes: {}", policy.allowance),
            format!(
                "operator_threshold_bytes: {}",
                policy.threshold
            ),
            "state: Other strands for this soul are suspended; their inbound messages remain durably queued.".to_string(),
            format!(
                "instruction: Inspect your full memory at {} and decide whether and how to organize it.",
                soulward()
            ),
            "advice: Use bounded reads or file-local processing. Do not echo the whole file into provider context.".to_string(),
            "boundary: Runtime has not changed the source and will never author, archive, summarize, or replace it.".to_string(),
            "resume_condition: Reducing the source below allowance_bytes automatically resumes queued strands.".to_string(),
            "</system_message>".to_string(),
        ]
        .join("\n"),
    )
}

fn contains(item: &Item, needle: &str) -> bool {
    matches!(item, Item::Message { content, .. } if content.contains(needle))
}

pub(super) fn maintain(service: &Service, maintenance_strand_id: &str) {
    if let drive::Outcome::Held(error) | drive::Outcome::Failed(error) = service.poke(
        maintenance_strand_id,
        "system",
        None,
        "soul_memory_maintenance",
    ) {
        eprintln!(
            "santi: memory maintenance drive failed strand={} code={}",
            maintenance_strand_id, error.code
        );
    }
}
