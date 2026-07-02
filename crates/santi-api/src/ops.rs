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
    /// Overall gate: schema at the expected version AND (memory absent, which is
    /// a fresh soul that falls back to the encoded default, OR memory readable).
    pub ok: bool,
}

/// Run the offline pre-check against the runtime paths resolved from env.
pub fn doctor() -> Result<DoctorReport, String> {
    doctor_at(&config::resolve_runtime_paths())
}

/// The pure pre-check over explicit paths (env-free, so it is unit-testable).
pub fn doctor_at(paths: &RuntimePaths) -> Result<DoctorReport, String> {
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
        ok: schema_ok && memory_ok,
    })
}

/// The result of an offline inbox seed.
#[derive(Debug, Clone, Serialize)]
pub struct SeedReport {
    pub strand_id: String,
    /// Durably enqueued (false ⟺ the inbox gate rejected it — the strand is far
    /// behind; the caller should treat this as a failure).
    pub accepted: bool,
    pub reason: Option<String>,
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
    let outcome = store.enqueue_inbox(
        strand_id,
        santi_core::MessageKind::SantiSystem,
        santi_core::MessageContent::text(text),
    )?;
    Ok(match outcome {
        santi_core::IngestOutcome::Accepted { strand_id } => SeedReport {
            strand_id,
            accepted: true,
            reason: None,
        },
        santi_core::IngestOutcome::Rejected { reason } => SeedReport {
            strand_id: strand_id.to_string(),
            accepted: false,
            reason: Some(reason),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn paths_under(root: &std::path::Path) -> RuntimePaths {
        RuntimePaths {
            database_path: root.join("runtime").join("db"),
            runtime_root: root.join("runtime"),
            execution_root: root.join("execution"),
        }
    }

    #[test]
    fn healthy_when_migrated_and_memory_readable() {
        let temp = tempfile::tempdir().expect("temp dir");
        let paths = paths_under(temp.path());
        // Open (migrates to the current schema) then write the default soul memory.
        santi_core::SantiStore::open(&paths.database_path).expect("open store");
        let memory = santi_core::soul_memory_file(&paths.runtime_root, santi_core::DEFAULT_SOUL_ID);
        std::fs::create_dir_all(memory.parent().unwrap()).unwrap();
        std::fs::write(&memory, "# I am the first secretary").unwrap();

        let report = doctor_at(&paths).expect("doctor");
        assert!(report.ok, "expected healthy: {report:?}");
        assert!(report.schema_ok);
        assert_eq!(report.schema_version, Some(santi_core::SCHEMA_VERSION));
        assert!(report.memory_present && report.memory_readable);
        assert!(report.memory_bytes > 0);
    }

    #[test]
    fn unhealthy_when_schema_stale() {
        let temp = tempfile::tempdir().expect("temp dir");
        let paths = paths_under(temp.path());
        std::fs::create_dir_all(paths.database_path.parent().unwrap()).unwrap();
        // A DB stamped at a stale version — a start WOULD wipe/migrate it.
        let conn = rusqlite::Connection::open(&paths.database_path).unwrap();
        conn.pragma_update(None, "user_version", 5u32).unwrap();
        drop(conn);

        let report = doctor_at(&paths).expect("doctor");
        assert!(!report.ok);
        assert!(!report.schema_ok);
        assert_eq!(report.schema_version, Some(5));
    }

    #[test]
    fn absent_memory_is_ok_but_absent_db_is_not() {
        let temp = tempfile::tempdir().expect("temp dir");
        let paths = paths_under(temp.path());
        santi_core::SantiStore::open(&paths.database_path).expect("open store");
        // No memory file written: a fresh soul falls back to the encoded default,
        // so memory-absent is healthy as long as the schema is current.
        let report = doctor_at(&paths).expect("doctor");
        assert!(report.ok, "absent memory should be fine: {report:?}");
        assert!(!report.memory_present);

        // But a DB that does not exist at all is not healthy (schema unknown).
        let missing = RuntimePaths {
            database_path: temp.path().join("void").join("db"),
            ..paths
        };
        let report = doctor_at(&missing).expect("doctor");
        assert!(!report.ok);
        assert_eq!(report.schema_version, None);
    }

    #[test]
    fn report_serializes_to_json() {
        let temp = tempfile::tempdir().expect("temp dir");
        let paths = paths_under(temp.path());
        santi_core::SantiStore::open(&paths.database_path).expect("open store");
        let report = doctor_at(&paths).expect("doctor");
        let json = serde_json::to_string(&report).expect("serialize");
        assert!(json.contains("\"schema_ok\""));
        let _ = PathBuf::from(&report.database_path);
    }

    #[test]
    fn seed_into_existing_strand_is_drainable_on_boot() {
        let temp = tempfile::tempdir().expect("temp dir");
        let paths = paths_under(temp.path());
        let strand_id = {
            let store = santi_core::SantiStore::open(&paths.database_path).expect("open");
            store.create_strand().expect("create strand").id
        };

        let report = inbox_seed_at(&paths, &strand_id, "you were upgrading — come look").unwrap();
        assert!(report.accepted);
        assert_eq!(report.strand_id, strand_id);

        // The seed is exactly what boot recovery scans for, and try_start_turn
        // (what a boot poke calls) drains it into a real turn carrying our text.
        let store = santi_core::SantiStore::open(&paths.database_path).expect("reopen");
        assert!(
            store
                .strands_with_pending_requests()
                .unwrap()
                .contains(&strand_id),
            "boot recovery would re-drive this strand"
        );
        // "strand_send" is exactly the trigger boot recovery uses (resume_pending).
        let started = store
            .try_start_turn(&strand_id, "strand_send", None)
            .unwrap()
            .expect("a turn starts by draining the seed");
        assert_eq!(started.drained_messages.len(), 1);
        assert_eq!(
            started.drained_messages[0].content_text,
            "you were upgrading — come look"
        );
        assert_eq!(
            started.drained_messages[0].message.message_kind,
            santi_core::MessageKind::SantiSystem
        );
    }

    #[test]
    fn seed_into_unknown_strand_errors_without_writing_orphan() {
        let temp = tempfile::tempdir().expect("temp dir");
        let paths = paths_under(temp.path());
        santi_core::SantiStore::open(&paths.database_path).expect("open");

        let err = inbox_seed_at(&paths, "ss_does_not_exist", "x").unwrap_err();
        assert!(err.contains("unknown strand"), "got: {err}");

        // Nothing was enqueued — boot recovery finds no orphan to choke on.
        let store = santi_core::SantiStore::open(&paths.database_path).expect("reopen");
        assert!(store.strands_with_pending_requests().unwrap().is_empty());
    }
}
