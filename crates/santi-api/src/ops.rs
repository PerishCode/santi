use std::fs;

use serde::Serialize;

use crate::config::RuntimePaths;
use crate::runtime::{self, Runtime};

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
    pub bytes: Option<usize>,
    pub budget_source: Option<String>,
    pub ok: bool,
    pub error: Option<String>,
}

pub fn doctor() -> Result<DoctorReport, String> {
    let held = runtime::held();
    held.paths.doctor_configured(held)
}

#[derive(Debug, Clone, Serialize)]
pub struct SeedReport {
    pub strand: String,
    pub accepted: bool,
    pub error: Option<santi_core::Fault>,
}

pub fn inbox_seed(strand: &str, text: &str) -> Result<SeedReport, String> {
    runtime::held().paths.inbox_seed(strand, text)
}

fn inbox_seed_existing_strand(
    store: &santi_core::SantiStore,
    strand: &str,
    text: &str,
) -> Result<SeedReport, String> {
    let outcome = store.enqueue_inbox_with_source(
        strand,
        santi_core::MessageKind::SantiSystem,
        santi_core::MessageContent::text(text),
        Some(santi_core::InboxSource::new("offline_inbox_seed")),
    )?;
    Ok(match outcome {
        santi_core::IngestOutcome::Accepted { receipt } => SeedReport {
            strand: receipt.strand,
            accepted: true,
            error: receipt.warning.map(|warning| *warning),
        },
        santi_core::IngestOutcome::Rejected { error } => SeedReport {
            strand: strand.to_string(),
            accepted: false,
            error: Some(*error),
        },
    })
}

impl RuntimePaths {
    pub fn doctor(&self) -> Result<DoctorReport, String> {
        self.doctor_report(None)
    }

    pub fn doctor_configured(&self, held: &Runtime) -> Result<DoctorReport, String> {
        let profile = Some(held.provider.clone());
        let provider = match held.provider_config() {
            Ok(provider) => ProviderDoctorReport {
                profile,
                kind: Some(provider.kind().to_string()),
                model: Some(provider.model().to_string()),
                bytes: Some(provider.bytes()),
                budget_source: Some("provider_config".to_string()),
                ok: true,
                error: None,
            },
            Err(error) => ProviderDoctorReport {
                profile,
                kind: None,
                model: None,
                bytes: None,
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

    pub fn inbox_seed(&self, strand: &str, text: &str) -> Result<SeedReport, String> {
        let store = santi_core::SantiStore::open(&self.database_path)?;
        if store.strand(strand)?.is_none() {
            return Err(format!("unknown strand: {strand}"));
        }
        inbox_seed_existing_strand(&store, strand, text)
    }

    pub fn inbox_seed_label(
        &self,
        soul: &str,
        label: &str,
        text: &str,
    ) -> Result<SeedReport, String> {
        let store = santi_core::SantiStore::open(&self.database_path)?;
        let strand = store.find_labeled_strand(soul, label)?;
        inbox_seed_existing_strand(&store, &strand.id, text)
    }
}
