use std::path::{Path, PathBuf};

use santi_api::config::Layout;

fn paths_under(root: &Path) -> Layout {
    Layout {
        database: root.join("runtime").join("db"),
        runtime: root.join("runtime"),
        execution: root.join("execution"),
    }
}

mod runtime {
    use super::*;

    #[test]
    fn reads() {
        let temp = tempfile::tempdir().expect("temp dir");
        let paths = paths_under(temp.path());
        santi_core::Store::open(&paths.database).expect("open store");
        let memory = santi_core::memoir(&paths.runtime, santi_core::GENESIS);
        std::fs::create_dir_all(memory.parent().unwrap()).unwrap();
        std::fs::write(&memory, "# memory").unwrap();

        let report = paths.doctor().expect("doctor");
        assert!(report.ok, "expected healthy: {report:?}");
        assert!(report.schema_ok);
        assert_eq!(report.schema_version, Some(santi_core::VERSION));
        assert!(report.memory_present && report.memory_readable);
        assert!(report.memory_bytes > 0);
    }

    #[test]
    fn rejects() {
        let temp = tempfile::tempdir().expect("temp dir");
        let paths = paths_under(temp.path());
        std::fs::create_dir_all(paths.database.parent().unwrap()).unwrap();
        let conn = rusqlite::Connection::open(&paths.database).unwrap();
        conn.pragma_update(None, "user_version", 5u32).unwrap();
        drop(conn);

        let report = paths.doctor().expect("doctor");
        assert!(!report.ok);
        assert!(!report.schema_ok);
        assert_eq!(report.schema_version, Some(5));
    }

    #[test]
    fn handles() {
        let temp = tempfile::tempdir().expect("temp dir");
        let paths = paths_under(temp.path());
        santi_core::Store::open(&paths.database).expect("open store");
        let report = paths.doctor().expect("doctor");
        assert!(report.ok, "absent memory should be fine: {report:?}");
        assert!(!report.memory_present);

        let missing = Layout {
            database: temp.path().join("void").join("db"),
            ..paths
        };
        let report = missing.doctor().expect("doctor");
        assert!(!report.ok);
        assert_eq!(report.schema_version, None);
    }

    #[test]
    fn serializes() {
        let temp = tempfile::tempdir().expect("temp dir");
        let paths = paths_under(temp.path());
        santi_core::Store::open(&paths.database).expect("open store");
        let report = paths.doctor().expect("doctor");
        let json = serde_json::to_string(&report).expect("serialize");
        assert!(json.contains("\"schema_ok\""));
        assert!(json.contains("\"provider\":null"));
        let _ = PathBuf::from(&report.database);
    }
}

mod seed {
    use super::*;

    #[test]
    fn boots() {
        let temp = tempfile::tempdir().expect("temp dir");
        let paths = paths_under(temp.path());
        let strand = {
            let store = santi_core::Store::open(&paths.database).expect("open");
            store.weave().expect("create strand").id
        };

        let report = paths.inbox_seed(&strand, "come look").unwrap();
        assert!(report.accepted);
        let store = santi_core::Store::open(&paths.database).expect("reopen");
        let started = store
            .tried(&strand, "strand_send", None)
            .unwrap()
            .expect("turn starts");
        assert_eq!(started.drained.len(), 1);
        assert_eq!(started.drained[0].text, "come look");
        assert_eq!(
            started.drained[0].message.kind,
            santi_core::message::Kind::SantiSystem
        );
    }

    #[test]
    fn labels() {
        let temp = tempfile::tempdir().expect("temp dir");
        let paths = paths_under(temp.path());
        santi_core::Store::open(&paths.database).expect("open");
        let label = "soul:soul_default:ops";

        let report = paths
            .inbox_seed_label(santi_core::GENESIS, label, "upgrade finished")
            .unwrap();
        assert!(report.accepted);
        let store = santi_core::Store::open(&paths.database).expect("reopen");
        let strand = store.strand(&report.strand).unwrap().expect("strand");
        assert_eq!(strand.label.as_deref(), Some(label));
        let started = store
            .tried(&report.strand, "strand_send", None)
            .unwrap()
            .expect("turn starts");
        assert_eq!(started.drained[0].text, "upgrade finished");
    }

    #[test]
    fn unknown() {
        let temp = tempfile::tempdir().expect("temp dir");
        let paths = paths_under(temp.path());
        santi_core::Store::open(&paths.database).expect("open");

        let error = paths.inbox_seed("ss_missing", "x").unwrap_err();
        assert!(error.contains("unknown strand"), "got: {error}");
        let store = santi_core::Store::open(&paths.database).expect("reopen");
        assert!(store.awaiting().unwrap().is_empty());
    }
}

mod doctor;
