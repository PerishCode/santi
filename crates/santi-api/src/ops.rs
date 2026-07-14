use std::fs;

use serde::Serialize;

use crate::config::{self, RuntimePaths};

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub database_path: String,
    pub database_exists: bool,
    pub schema_version: Option<u32>,
    pub expected_schema_version: u32,
    pub schema_ok: bool,
    pub default_soul_id: String,
    pub memory_path: String,
    pub memory_present: bool,
    pub memory_readable: bool,
    pub memory_bytes: u64,
    pub provider: Option<ProviderDoctorReport>,
    pub ok: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderDoctorReport {
    pub profile: Option<String>,
    pub kind: Option<String>,
    pub model: Option<String>,
    pub input_budget_bytes: Option<usize>,
    pub budget_source: Option<String>,
    pub ok: bool,
    pub error: Option<String>,
}

pub fn doctor() -> Result<DoctorReport, String> {
    let service = config::ConfigService::from_args(["santi", "serve"].map(str::to_string))?;
    config::resolve_runtime_paths().doctor_configured(&service)
}

#[derive(Debug, Clone, Serialize)]
pub struct SeedReport {
    pub strand_id: String,
    pub accepted: bool,
    pub error: Option<santi_core::SantiError>,
}

pub fn inbox_seed(strand_id: &str, text: &str) -> Result<SeedReport, String> {
    config::resolve_runtime_paths().inbox_seed(strand_id, text)
}

fn inbox_seed_existing_strand(
    store: &santi_core::SantiStore,
    strand_id: &str,
    text: &str,
) -> Result<SeedReport, String> {
    let outcome = store.enqueue_inbox_with_source(
        strand_id,
        santi_core::MessageKind::SantiSystem,
        santi_core::MessageContent::text(text),
        Some(santi_core::InboxSource::new("offline_inbox_seed")),
    )?;
    Ok(match outcome {
        santi_core::IngestOutcome::Accepted { receipt } => SeedReport {
            strand_id: receipt.strand_id,
            accepted: true,
            error: receipt.warning.map(|warning| *warning),
        },
        santi_core::IngestOutcome::Rejected { error } => SeedReport {
            strand_id: strand_id.to_string(),
            accepted: false,
            error: Some(*error),
        },
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct ImReplyReport {
    pub participant_id: String,
    pub seq: i64,
    pub turn_id: Option<String>,
    pub delivery_mode: Option<santi_core::ImDeliveryMode>,
    pub deduplicated: bool,
}

pub fn im_reply(strand_id: &str, content: &str) -> Result<ImReplyReport, String> {
    let turn_id = std::env::var("SANTI_TURN_ID")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    config::resolve_runtime_paths().im_reply_turn(strand_id, turn_id.as_deref(), content)
}

impl RuntimePaths {
    pub fn doctor(&self) -> Result<DoctorReport, String> {
        self.doctor_report(None)
    }

    pub fn doctor_configured(
        &self,
        config: &config::ConfigService,
    ) -> Result<DoctorReport, String> {
        let profile = config.provider_name().ok();
        let provider = match config.provider_config() {
            Ok(provider) => ProviderDoctorReport {
                profile,
                kind: Some(provider.kind().to_string()),
                model: Some(provider.model().to_string()),
                input_budget_bytes: Some(provider.input_budget_bytes()),
                budget_source: Some("provider_config".to_string()),
                ok: true,
                error: None,
            },
            Err(error) => ProviderDoctorReport {
                profile,
                kind: None,
                model: None,
                input_budget_bytes: None,
                budget_source: None,
                ok: false,
                error: Some(error.to_string()),
            },
        };
        self.doctor_report(Some(provider))
    }

    fn doctor_report(
        &self,
        provider: Option<ProviderDoctorReport>,
    ) -> Result<DoctorReport, String> {
        let database_exists = self.database_path.exists();
        let schema_version = santi_core::read_schema_version(&self.database_path)?;
        let schema_ok = schema_version == Some(santi_core::SCHEMA_VERSION);
        let memory_path =
            santi_core::soul_memory_file(&self.runtime_root, santi_core::DEFAULT_SOUL_ID);
        let memory_present = memory_path.exists();
        let (memory_readable, memory_bytes) = match fs::read(&memory_path) {
            Ok(bytes) => (true, bytes.len() as u64),
            Err(_) => (false, 0),
        };
        let memory_ok = !memory_present || memory_readable;
        let provider_ok = provider.as_ref().is_none_or(|provider| provider.ok);

        Ok(DoctorReport {
            database_path: self.database_path.display().to_string(),
            database_exists,
            schema_version,
            expected_schema_version: santi_core::SCHEMA_VERSION,
            schema_ok,
            default_soul_id: santi_core::DEFAULT_SOUL_ID.to_string(),
            memory_path: memory_path.display().to_string(),
            memory_present,
            memory_readable,
            memory_bytes,
            provider,
            ok: schema_ok && memory_ok && provider_ok,
        })
    }

    pub fn inbox_seed(&self, strand_id: &str, text: &str) -> Result<SeedReport, String> {
        let store = santi_core::SantiStore::open(&self.database_path)?;
        if store.strand(strand_id)?.is_none() {
            return Err(format!("unknown strand: {strand_id}"));
        }
        inbox_seed_existing_strand(&store, strand_id, text)
    }

    pub fn inbox_seed_label(
        &self,
        soul_id: &str,
        label: &str,
        text: &str,
    ) -> Result<SeedReport, String> {
        let store = santi_core::SantiStore::open(&self.database_path)?;
        let strand = store.find_labeled_strand(soul_id, label)?;
        inbox_seed_existing_strand(&store, &strand.id, text)
    }

    pub fn im_reply(&self, strand_id: &str, content: &str) -> Result<ImReplyReport, String> {
        self.im_reply_turn(strand_id, None, content)
    }

    pub fn im_reply_turn(
        &self,
        strand_id: &str,
        turn_id: Option<&str>,
        content: &str,
    ) -> Result<ImReplyReport, String> {
        let store = santi_core::SantiStore::open(&self.database_path)?;
        let (entry, inserted) = match turn_id {
            Some(turn_id) => store.enqueue_turn_reply(santi_core::Reply {
                strand: strand_id,
                turn: turn_id,
                message: None,
                content,
                mode: santi_core::ImDeliveryMode::Explicit,
            })?,
            None => {
                let participant_id = store
                    .im_participant_for_strand(strand_id)?
                    .ok_or_else(|| format!("strand {strand_id} is not an IM conversation"))?;
                (
                    store.enqueue_im_inbox(&participant_id, Some(strand_id), content)?,
                    true,
                )
            }
        };
        Ok(ImReplyReport {
            participant_id: entry.participant_id.clone(),
            seq: entry.seq,
            turn_id: entry.turn_id,
            delivery_mode: entry.delivery_mode,
            deduplicated: !inserted,
        })
    }
}
