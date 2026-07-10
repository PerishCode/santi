use std::path::{Path, PathBuf};

use santi_api::{
    config::RuntimePaths,
    ops::{doctor_at, im_reply_at, inbox_seed_at, inbox_seed_label_at},
};

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

    let report = doctor_at(&paths).expect("doctor");
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

    let report = doctor_at(&paths).expect("doctor");
    assert!(!report.ok);
    assert!(!report.schema_ok);
    assert_eq!(report.schema_version, Some(5));
}

#[test]
fn doctor_handles_absence() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    santi_core::SantiStore::open(&paths.database_path).expect("open store");
    let report = doctor_at(&paths).expect("doctor");
    assert!(report.ok, "absent memory should be fine: {report:?}");
    assert!(!report.memory_present);

    let missing = RuntimePaths {
        database_path: temp.path().join("void").join("db"),
        ..paths
    };
    let report = doctor_at(&missing).expect("doctor");
    assert!(!report.ok);
    assert_eq!(report.schema_version, None);
}

#[test]
fn doctor_serializes() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    santi_core::SantiStore::open(&paths.database_path).expect("open store");
    let report = doctor_at(&paths).expect("doctor");
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

    let report = inbox_seed_at(&paths, &strand_id, "come look").unwrap();
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

    let report = inbox_seed_label_at(
        &paths,
        santi_core::DEFAULT_SOUL_ID,
        label,
        "upgrade finished",
    )
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

    let report = im_reply_at(&paths, &strand_id, "ok").unwrap();
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
    let error = im_reply_at(&paths, &strand_id, "x").unwrap_err();
    assert!(error.contains("not an IM conversation"), "got: {error}");
}

#[test]
fn seed_rejects_unknown() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    santi_core::SantiStore::open(&paths.database_path).expect("open");

    let error = inbox_seed_at(&paths, "ss_missing", "x").unwrap_err();
    assert!(error.contains("unknown strand"), "got: {error}");
    let store = santi_core::SantiStore::open(&paths.database_path).expect("reopen");
    assert!(store.strands_with_pending_requests().unwrap().is_empty());
}
