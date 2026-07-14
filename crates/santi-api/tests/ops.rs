use std::path::{Path, PathBuf};

use santi_api::config::RuntimePaths;

fn paths_under(root: &Path) -> RuntimePaths {
    RuntimePaths {
        database_path: root.join("runtime").join("db"),
        runtime_root: root.join("runtime"),
        execution_root: root.join("execution"),
    }
}

#[test]
fn doctor_reads_runtime() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    santi_core::SantiStore::open(&paths.database_path).expect("open store");
    let memory = santi_core::soul_memory_file(&paths.runtime_root, santi_core::DEFAULT_SOUL_ID);
    std::fs::create_dir_all(memory.parent().unwrap()).unwrap();
    std::fs::write(&memory, "# memory").unwrap();

    let report = paths.doctor().expect("doctor");
    assert!(report.ok, "expected healthy: {report:?}");
    assert!(report.schema_ok);
    assert_eq!(report.schema_version, Some(santi_core::SCHEMA_VERSION));
    assert!(report.memory_present && report.memory_readable);
    assert!(report.memory_bytes > 0);
}

#[test]
fn doctor_rejects_stale() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    std::fs::create_dir_all(paths.database_path.parent().unwrap()).unwrap();
    let conn = rusqlite::Connection::open(&paths.database_path).unwrap();
    conn.pragma_update(None, "user_version", 5u32).unwrap();
    drop(conn);

    let report = paths.doctor().expect("doctor");
    assert!(!report.ok);
    assert!(!report.schema_ok);
    assert_eq!(report.schema_version, Some(5));
}

#[test]
fn doctor_handles_absence() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    santi_core::SantiStore::open(&paths.database_path).expect("open store");
    let report = paths.doctor().expect("doctor");
    assert!(report.ok, "absent memory should be fine: {report:?}");
    assert!(!report.memory_present);

    let missing = RuntimePaths {
        database_path: temp.path().join("void").join("db"),
        ..paths
    };
    let report = missing.doctor().expect("doctor");
    assert!(!report.ok);
    assert_eq!(report.schema_version, None);
}

#[test]
fn doctor_serializes() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    santi_core::SantiStore::open(&paths.database_path).expect("open store");
    let report = paths.doctor().expect("doctor");
    let json = serde_json::to_string(&report).expect("serialize");
    assert!(json.contains("\"schema_ok\""));
    assert!(json.contains("\"provider\":null"));
    let _ = PathBuf::from(&report.database_path);
}

#[test]
fn seed_drains_on_boot() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    let strand_id = {
        let store = santi_core::SantiStore::open(&paths.database_path).expect("open");
        store.create_strand().expect("create strand").id
    };

    let report = paths.inbox_seed(&strand_id, "come look").unwrap();
    assert!(report.accepted);
    let store = santi_core::SantiStore::open(&paths.database_path).expect("reopen");
    let started = store
        .try_start_turn(&strand_id, "strand_send", None)
        .unwrap()
        .expect("turn starts");
    assert_eq!(started.drained_messages.len(), 1);
    assert_eq!(started.drained_messages[0].content_text, "come look");
    assert_eq!(
        started.drained_messages[0].message.message_kind,
        santi_core::MessageKind::SantiSystem
    );
}

#[test]
fn labeled_seed_drains() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    santi_core::SantiStore::open(&paths.database_path).expect("open");
    let label = "soul:soul_default:ops";

    let report = paths
        .inbox_seed_label(santi_core::DEFAULT_SOUL_ID, label, "upgrade finished")
        .unwrap();
    assert!(report.accepted);
    let store = santi_core::SantiStore::open(&paths.database_path).expect("reopen");
    let strand = store.strand(&report.strand_id).unwrap().expect("strand");
    assert_eq!(strand.external_label.as_deref(), Some(label));
    let started = store
        .try_start_turn(&report.strand_id, "strand_send", None)
        .unwrap()
        .expect("turn starts");
    assert_eq!(started.drained_messages[0].content_text, "upgrade finished");
}

#[test]
fn im_reply_delivers() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    let strand_id = {
        let store = santi_core::SantiStore::open(&paths.database_path).expect("open");
        store.ensure_im_participant("operator", "human").unwrap();
        store
            .find_labeled_strand(santi_core::DEFAULT_SOUL_ID, "im:operator")
            .expect("conversation strand")
            .id
    };

    let report = paths.im_reply(&strand_id, "ok").unwrap();
    assert_eq!(report.participant_id, "operator");
    let store = santi_core::SantiStore::open(&paths.database_path).expect("reopen");
    let entries = store.poll_im_inbox("operator", 0).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].content, "ok");
    assert_eq!(entries[0].from_ref.as_deref(), Some(strand_id.as_str()));
}

#[test]
fn im_reply_rejects_plain() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    let strand_id = {
        let store = santi_core::SantiStore::open(&paths.database_path).expect("open");
        store.create_strand().expect("create strand").id
    };
    let error = paths.im_reply(&strand_id, "x").unwrap_err();
    assert!(error.contains("not an IM conversation"), "got: {error}");
}

#[test]
fn turn_reply_deduplicates() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    let (strand_id, turn_id) = {
        let store = santi_core::SantiStore::open(&paths.database_path).expect("open");
        store.ensure_im_participant("operator", "human").unwrap();
        let strand = store
            .find_labeled_strand(santi_core::DEFAULT_SOUL_ID, "im:operator")
            .unwrap();
        store
            .enqueue_inbox(
                &strand.id,
                santi_core::MessageKind::Text,
                santi_core::MessageContent::text("hello"),
            )
            .unwrap();
        let turn = store
            .try_start_turn(&strand.id, "strand_send", None)
            .unwrap()
            .expect("turn")
            .turn;
        (strand.id, turn.id)
    };

    let first = paths
        .im_reply_turn(&strand_id, Some(&turn_id), "early")
        .unwrap();
    assert!(!first.deduplicated);
    assert_eq!(first.turn_id.as_deref(), Some(turn_id.as_str()));
    assert_eq!(
        first.delivery_mode,
        Some(santi_core::ImDeliveryMode::Explicit)
    );
    let second = paths
        .im_reply_turn(&strand_id, Some(&turn_id), "duplicate")
        .unwrap();
    assert!(second.deduplicated);
    assert_eq!(second.seq, first.seq);
}

#[test]
fn seed_rejects_unknown() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    santi_core::SantiStore::open(&paths.database_path).expect("open");

    let error = paths.inbox_seed("ss_missing", "x").unwrap_err();
    assert!(error.contains("unknown strand"), "got: {error}");
    let store = santi_core::SantiStore::open(&paths.database_path).expect("reopen");
    assert!(store.strands_with_pending_requests().unwrap().is_empty());
}
