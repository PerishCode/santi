use std::fs;

use santi_provider::ProviderItem;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::{
    ErrorScope, ErrorSource, InboxSource, IncidentDraft, IngestOutcome, MessageContent,
    MessageKind, Strand, catalog, soul_memory_uri,
};

use super::super::{Service, drive};

const FALLBACK_INPUT_BUDGET_BYTES: usize = 500_000;
pub(super) const MEMORY_MAINTENANCE_LABEL: &str = "santi:memory:maintenance";

#[derive(Clone, Copy)]
pub(in crate::service) struct Policy {
    pub(in crate::service) allowance_bytes: usize,
    operator_threshold_bytes: usize,
}

pub(super) enum Gate {
    Allow,
    Pause { maintenance_strand_id: String },
}

struct Snapshot {
    source_bytes: usize,
    sha256: String,
}

impl Service {
    pub(in crate::service) fn soul_memory_policy(&self) -> Policy {
        let input_budget_bytes = self
            .provider
            .metadata()
            .context_budget
            .map_or(FALLBACK_INPUT_BUDGET_BYTES, |budget| {
                budget.input_budget_bytes
            });
        let allowance_bytes = (input_budget_bytes / 2).max(1);
        let operator_threshold_bytes =
            (input_budget_bytes.saturating_mul(3) / 4).max(allowance_bytes.saturating_add(1));
        Policy {
            allowance_bytes,
            operator_threshold_bytes,
        }
    }

    pub(super) fn memory_drive_gate(&self, strand: &Strand) -> Result<Gate, String> {
        let _guard = self.memory_pressure_lock.lock().unwrap();
        let policy = self.soul_memory_policy();
        let snapshot = self.soul_memory_snapshot(&strand.soul_id)?;
        self.reconcile_memory_intervention(&strand.soul_id, &snapshot, policy)?;
        if snapshot.source_bytes <= policy.allowance_bytes {
            return Ok(Gate::Allow);
        }

        let maintenance = self
            .store
            .find_labeled_strand(&strand.soul_id, MEMORY_MAINTENANCE_LABEL)?;
        self.ensure_memory_maintenance_prompt(&maintenance, &snapshot, policy)?;
        if strand.id == maintenance.id {
            Ok(Gate::Allow)
        } else {
            Ok(Gate::Pause {
                maintenance_strand_id: maintenance.id,
            })
        }
    }

    pub(super) fn resume_after_memory_maintenance(&self, strand_id: &str) {
        if let Err(error) = self.resume_memory_soul(strand_id) {
            eprintln!("santi: soul memory relief scan failed strand_id={strand_id}: {error}");
        }
    }

    fn resume_memory_soul(&self, strand_id: &str) -> Result<(), String> {
        let Some(maintenance) = self.store.strand(strand_id)? else {
            return Ok(());
        };
        if maintenance.external_label.as_deref() != Some(MEMORY_MAINTENANCE_LABEL) {
            return Ok(());
        }
        let policy = self.soul_memory_policy();
        let snapshot = self.soul_memory_snapshot(&maintenance.soul_id)?;
        self.reconcile_memory_intervention(&maintenance.soul_id, &snapshot, policy)?;
        if snapshot.source_bytes > policy.allowance_bytes {
            return Ok(());
        }

        for pending_id in self.store.strands_with_pending_requests()? {
            if pending_id == strand_id {
                continue;
            }
            let Some(pending) = self.store.strand(&pending_id)? else {
                continue;
            };
            if pending.soul_id == maintenance.soul_id {
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

    fn soul_memory_snapshot(&self, soul_id: &str) -> Result<Snapshot, String> {
        let path = self.soul_memory_file(soul_id);
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error.to_string()),
        };
        Ok(Snapshot {
            source_bytes: bytes.len(),
            sha256: hex::encode(Sha256::digest(&bytes)),
        })
    }

    fn ensure_memory_maintenance_prompt(
        &self,
        maintenance: &Strand,
        snapshot: &Snapshot,
        policy: Policy,
    ) -> Result<(), String> {
        let fingerprint = format!("source_sha256: {}", snapshot.sha256);
        let recorded = self
            .store
            .strand_messages(&maintenance.id)?
            .iter()
            .any(|message| message.content_text.contains(&fingerprint));
        let pending = self
            .store
            .pending_provider_items(&maintenance.id)?
            .iter()
            .any(|item| provider_item_contains(item, &fingerprint));
        if recorded || pending {
            return Ok(());
        }

        let outcome = self.store.enqueue_inbox_while_suspended(
            &maintenance.id,
            MessageKind::SantiSystem,
            memory_maintenance_metaprompt(snapshot, policy),
            Some(InboxSource::new("runtime_memory_pressure").with_ref(maintenance.soul_id.clone())),
        )?;
        match outcome {
            IngestOutcome::Accepted { .. } => Ok(()),
            IngestOutcome::Rejected { error } => Err(format!(
                "memory maintenance metaprompt was rejected: {}",
                error.message
            )),
        }
    }

    fn reconcile_memory_intervention(
        &self,
        soul_id: &str,
        snapshot: &Snapshot,
        policy: Policy,
    ) -> Result<(), String> {
        let incident_key = memory_intervention_incident_key(soul_id);
        let active = self.store.active_error_incident(&incident_key)?;
        let mutated = if snapshot.source_bytes > policy.operator_threshold_bytes {
            if active.is_some() {
                false
            } else {
                self.store.open_error_incident(IncidentDraft {
                    incident_key,
                    descriptor: catalog::SOUL_MEMORY_INTERVENTION_REQUIRED,
                    scope: ErrorScope::new("soul", soul_id),
                    source: ErrorSource::new("santi-core", "soul_memory_pressure"),
                    message: "soul memory exceeds the human-intervention threshold".to_string(),
                    context: json!({
                        "schema": "santi.error.soul_memory.v1",
                        "source": soul_memory_uri(),
                        "source_bytes": snapshot.source_bytes,
                        "allowance_bytes": policy.allowance_bytes,
                        "operator_threshold_bytes": policy.operator_threshold_bytes,
                        "maintenance_label": MEMORY_MAINTENANCE_LABEL,
                        "runtime_mutated_memory": false,
                    }),
                })?;
                true
            }
        } else if active.is_some() {
            self.store.resolve_error_incident(
                &incident_key,
                "soul_memory_remeasured",
                json!({
                    "schema": "santi.error.soul_memory.resolution.v1",
                    "source_bytes": snapshot.source_bytes,
                    "allowance_bytes": policy.allowance_bytes,
                    "operator_threshold_bytes": policy.operator_threshold_bytes,
                }),
            )?
        } else {
            false
        };
        if mutated {
            self.dispatch_error_events();
        }
        Ok(())
    }
}

fn memory_maintenance_metaprompt(snapshot: &Snapshot, policy: Policy) -> MessageContent {
    MessageContent::text(
        [
            "<system_message>".to_string(),
            "kind: soul_memory_maintenance".to_string(),
            "scope: soul".to_string(),
            "wake: true".to_string(),
            format!("source: {}", soul_memory_uri()),
            format!("source_sha256: {}", snapshot.sha256),
            format!("source_bytes: {}", snapshot.source_bytes),
            format!("allowance_bytes: {}", policy.allowance_bytes),
            format!(
                "operator_threshold_bytes: {}",
                policy.operator_threshold_bytes
            ),
            "state: Other strands for this soul are suspended; their inbound messages remain durably queued.".to_string(),
            format!(
                "instruction: Inspect your full memory at {} and decide whether and how to organize it.",
                soul_memory_uri()
            ),
            "advice: Use bounded reads or file-local processing. Do not echo the whole file into provider context.".to_string(),
            "boundary: Runtime has not changed the source and will never author, archive, summarize, or replace it.".to_string(),
            "resume_condition: Reducing the source below allowance_bytes automatically resumes queued strands.".to_string(),
            "</system_message>".to_string(),
        ]
        .join("\n"),
    )
}

fn provider_item_contains(item: &ProviderItem, needle: &str) -> bool {
    matches!(item, ProviderItem::Message { content, .. } if content.contains(needle))
}

fn memory_intervention_incident_key(soul_id: &str) -> String {
    format!(
        "{}:soul:{soul_id}",
        catalog::SOUL_MEMORY_INTERVENTION_REQUIRED.code
    )
}

pub(super) fn drive_maintenance(service: &Service, maintenance_strand_id: &str) {
    if let drive::Outcome::Held(error) | drive::Outcome::Failed(error) = service.poke(
        maintenance_strand_id,
        "system",
        None,
        "soul_memory_maintenance",
    ) {
        eprintln!(
            "santi: memory maintenance drive failed strand_id={} code={}",
            maintenance_strand_id, error.code
        );
    }
}
