//! Local (non-HTTP) operator commands. Unlike the client commands, these do NOT
//! reach a running runtime over HTTP — they act directly on the on-disk store
//! and runtime files. They exist for the self-upgrade lifecycle (PHASE-07),
//! where the service is often stopped, and are grouped here so `santi-api`
//! stays the single owner of path resolution (`config::resolve_runtime_paths`).

use std::fs;

use serde::Serialize;

use crate::config::{self, RuntimePaths};

/// A read-only pre-check of the on-disk store + default soul memory. Pure reads:
/// it never opens the store (which would migrate/wipe) and is safe against a
/// live or stopped service. The soul-deep health "come up + coherent" contract
/// is confirmed functionally elsewhere (PHASE-07 open #2); this is the cheap
/// deterministic half — "is the store at the expected schema and is the default
/// soul's memory readable".
#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub database_path: String,
    pub database_exists: bool,
    /// The DB's `user_version`, or null when the DB file does not exist yet.
    pub schema_version: Option<u32>,
    pub expected_schema_version: u32,
    /// The DB is present and already at the version this binary expects (so a
    /// start would NOT wipe/migrate).
    pub schema_ok: bool,
    pub default_soul_id: String,
    pub memory_path: String,
    pub memory_present: bool,
    pub memory_readable: bool,
    pub memory_bytes: u64,
    /// Present for the operator-facing doctor command. Internal storage-only
    /// upgrade checks omit this and retain their existing scope.
    pub provider: Option<ProviderDoctorReport>,
    /// Overall gate: schema at the expected version AND (memory absent, which is
    /// a fresh soul that falls back to the encoded default, OR memory readable).
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

/// Run the offline pre-check against the runtime paths resolved from env.
pub fn doctor() -> Result<DoctorReport, String> {
    let service = config::ConfigService::from_args(["santi", "serve"].map(str::to_string))?;
    doctor_configured_at(&config::resolve_runtime_paths(), &service)
}

/// The pure pre-check over explicit paths (env-free, so it is unit-testable).
pub fn doctor_at(paths: &RuntimePaths) -> Result<DoctorReport, String> {
    doctor_report_at(paths, None)
}

pub fn doctor_configured_at(
    paths: &RuntimePaths,
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
    doctor_report_at(paths, Some(provider))
}

fn doctor_report_at(
    paths: &RuntimePaths,
    provider: Option<ProviderDoctorReport>,
) -> Result<DoctorReport, String> {
    let database_exists = paths.database_path.exists();
    let schema_version = santi_core::read_schema_version(&paths.database_path)?;
    let schema_ok = schema_version == Some(santi_core::SCHEMA_VERSION);

    let memory_path =
        santi_core::soul_memory_file(&paths.runtime_root, santi_core::DEFAULT_SOUL_ID);
    let memory_present = memory_path.exists();
    let (memory_readable, memory_bytes) = match fs::read(&memory_path) {
        Ok(bytes) => (true, bytes.len() as u64),
        Err(_) => (false, 0),
    };
    // Absent memory is fine (a fresh soul falls back to the encoded default);
    // present-but-unreadable is the failure — the soul's continuity would break.
    let memory_ok = !memory_present || memory_readable;
    let provider_ok = provider.as_ref().is_none_or(|provider| provider.ok);

    Ok(DoctorReport {
        database_path: paths.database_path.display().to_string(),
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

/// The result of an offline inbox seed.
#[derive(Debug, Clone, Serialize)]
pub struct SeedReport {
    pub strand_id: String,
    /// Durably enqueued (false ⟺ the inbox gate rejected it — the strand is far
    /// behind; the caller should treat this as a failure).
    pub accepted: bool,
    pub error: Option<santi_core::SantiError>,
}

/// Enqueue one `santi_system` record into a strand's durable inbox WITHOUT a
/// running service (a direct MQ producer). Used by the self-upgrade flow to seed
/// the "you were upgrading — come look" record before starting the final version,
/// so boot recovery drains it and the soul wakes into the result (PHASE-07).
///
/// Opens the store with THIS binary (so it migrates to this binary's schema —
/// seed with the FINAL version). The target strand MUST already exist: enqueuing
/// into an unknown strand would leave an inbox row that boot recovery can never
/// turn into a turn, so we reject instead of writing an orphan.
///
/// This is intentionally an offline producer, not live external ingress: it
/// respects active incidents, but candidate budget admission happens when
/// the service later resumes/drives the pending inbox.
pub fn inbox_seed(strand_id: &str, text: &str) -> Result<SeedReport, String> {
    inbox_seed_at(&config::resolve_runtime_paths(), strand_id, text)
}

pub fn inbox_seed_at(
    paths: &RuntimePaths,
    strand_id: &str,
    text: &str,
) -> Result<SeedReport, String> {
    let store = santi_core::SantiStore::open(&paths.database_path)?;
    if store.strand(strand_id)?.is_none() {
        return Err(format!("unknown strand: {strand_id}"));
    }
    inbox_seed_existing_strand(&store, strand_id, text)
}

/// Enqueue into the strand anchored by a stable label, creating it if it is
/// missing. This is the offline twin of webhook per-thread ingest's
/// label→strand materialization: the label is the durable routing anchor, while
/// the concrete strand id is a replaceable room.
pub fn inbox_seed_label_at(
    paths: &RuntimePaths,
    soul_id: &str,
    label: &str,
    text: &str,
) -> Result<SeedReport, String> {
    let store = santi_core::SantiStore::open(&paths.database_path)?;
    let strand = store.find_labeled_strand(soul_id, label)?;
    inbox_seed_existing_strand(&store, &strand.id, text)
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

/// The result of an offline early IM reply into a participant inbox.
#[derive(Debug, Clone, Serialize)]
pub struct ImReplyReport {
    pub participant_id: String,
    /// The delivered entry's cursor seq (what the participant's poll advances past).
    pub seq: i64,
    /// Present when the command runs in a provider turn's ambient shell.
    pub turn_id: Option<String>,
    pub delivery_mode: Option<santi_core::ImDeliveryMode>,
    /// True when this turn had already delivered a reply and the existing entry
    /// was returned instead of enqueueing a duplicate.
    pub deduplicated: bool,
}

/// Deliver the soul's reply into an IM participant's passive inbox WITHOUT reaching
/// the running runtime over HTTP — a direct store write (the mirror of `inbox_seed`),
/// so a soul replying MID-TURN never re-enters the turn-holding server (no self-call
/// deadlock). `strand_id` is the ambient current conversation (`SANTI_STRAND_ID` in
/// the soul's shell env); it must be an IM conversation (an `im:<participant>` label)
/// — the reply-routing correlation resolves the target participant from it.
pub fn im_reply(strand_id: &str, content: &str) -> Result<ImReplyReport, String> {
    let turn_id = std::env::var("SANTI_TURN_ID")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    im_reply_turn_at(
        &config::resolve_runtime_paths(),
        strand_id,
        turn_id.as_deref(),
        content,
    )
}

pub fn im_reply_at(
    paths: &RuntimePaths,
    strand_id: &str,
    content: &str,
) -> Result<ImReplyReport, String> {
    im_reply_turn_at(paths, strand_id, None, content)
}

pub fn im_reply_turn_at(
    paths: &RuntimePaths,
    strand_id: &str,
    turn_id: Option<&str>,
    content: &str,
) -> Result<ImReplyReport, String> {
    let store = santi_core::SantiStore::open(&paths.database_path)?;
    let (entry, inserted) = match turn_id {
        Some(turn_id) => store.enqueue_turn_reply(
            strand_id,
            turn_id,
            None,
            content,
            santi_core::ImDeliveryMode::Explicit,
        )?,
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
