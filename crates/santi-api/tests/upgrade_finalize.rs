use std::path::Path;

use santi_api::{
    config::RuntimePaths,
    upgrade::{
        RecoveryStatus, UpgradeFailure, UpgradeFinalizeRequest, UpgradeReadiness, UpgradeStage,
        UpgradeTerminal, finalize_at, seed_come_look_at,
    },
};
use santi_core::{ErrorScope, IncidentStatus, SantiStore};

#[test]
fn old_request_defaults() {
    let request: UpgradeFinalizeRequest = serde_json::from_value(serde_json::json!({
        "protocol_version": 1,
        "attempt_id": "upgrade_old_runner",
        "deb": "/tmp/santi.deb",
        "terminal": {"terminal": "upgraded"},
        "wake": true,
        "soul_id": "soul_default",
        "configured_strand_id": null,
    }))
    .expect("decode old runner request");
    assert_eq!(request.readiness, UpgradeReadiness::Ready);
    assert!(matches!(request.terminal, UpgradeTerminal::Upgraded));
}

#[test]
fn stable_label_seeds() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    SantiStore::open(&paths.database_path).expect("open");

    let outcome = seed_come_look_at(
        &paths,
        santi_core::DEFAULT_SOUL_ID,
        Some("ss_stale"),
        "come look",
    )
    .expect("seed via stable label");
    assert!(outcome.warnings.is_empty());

    let store = SantiStore::open(&paths.database_path).expect("reopen");
    let strand = store.strand(&outcome.strand_id).unwrap().expect("strand");
    assert_eq!(
        strand.external_label.as_deref(),
        Some("soul:soul_default:ops")
    );
    let started = store
        .try_start_turn(&outcome.strand_id, "strand_send", None)
        .unwrap()
        .expect("turn starts");
    assert_eq!(started.drained_messages[0].content_text, "come look");
}

#[test]
fn rollback_detail_not_projected() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    let report = finalize_at(
        &paths,
        request(UpgradeTerminal::RolledBack {
            failure: UpgradeFailure {
                stage: UpgradeStage::Install,
                detail: "PACKAGE_SECRET_DETAIL".to_string(),
                recovery: RecoveryStatus::PreviousVersionRestored,
            },
        }),
    )
    .expect("finalize rollback");

    assert!(report.seeded);
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].code, "runtime.upgrade.failed");
    assert_eq!(report.errors[0].context["detail"], "PACKAGE_SECRET_DETAIL");
    assert!(!report.record.unwrap().contains("PACKAGE_SECRET_DETAIL"));

    let store = SantiStore::open(&paths.database_path).expect("open store");
    let incidents = store
        .error_incidents(&ErrorScope::new("runtime", "default"), 10)
        .expect("runtime errors");
    assert_eq!(incidents.len(), 1);
    assert_eq!(incidents[0].status, IncidentStatus::Active);
    assert!(!incidents[0].exposure.model);
}

#[test]
fn success_resolves_incident() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    finalize_at(
        &paths,
        request(UpgradeTerminal::RolledBack {
            failure: UpgradeFailure {
                stage: UpgradeStage::TrialProbe,
                detail: "unhealthy".to_string(),
                recovery: RecoveryStatus::PreviousVersionRestored,
            },
        }),
    )
    .expect("finalize rollback");
    let recovered =
        finalize_at(&paths, request(UpgradeTerminal::Upgraded)).expect("finalize success");
    assert!(recovered.errors.is_empty());

    let store = SantiStore::open(&paths.database_path).expect("open store");
    let incidents = store
        .error_incidents(&ErrorScope::new("runtime", "default"), 10)
        .expect("runtime errors");
    assert_eq!(incidents.len(), 1);
    assert_eq!(incidents[0].status, IncidentStatus::Resolved);
    assert_eq!(incidents[0].revision, 2);
    assert_eq!(
        incidents[0].resolved_by.as_deref(),
        Some("upgrade.succeeded")
    );
}

#[test]
fn full_handover_is_idempotent() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    let seeded = seed_come_look_at(&paths, santi_core::DEFAULT_SOUL_ID, None, "existing wake")
        .expect("initial seed");
    let store = SantiStore::open(&paths.database_path).expect("open store");
    let conn = rusqlite::Connection::open(&paths.database_path).expect("open sqlite");
    conn.execute(
        r#"
        WITH RECURSIVE seq(n) AS (
          VALUES(1)
          UNION ALL
          SELECT n + 1 FROM seq WHERE n < 499
        )
        INSERT INTO strand_inbox (
          id, strand_id, message_kind, content,
          source_type, source_ref, source_metadata, created_at
        )
        SELECT 'inbox_fixture_' || n, ?1, 'santi_system', '{}',
               NULL, NULL, NULL, 'fixture'
        FROM seq
        "#,
        [&seeded.strand_id],
    )
    .expect("fill inbox fixture");

    let request = request(UpgradeTerminal::RolledBack {
        failure: UpgradeFailure {
            stage: UpgradeStage::Install,
            detail: "bad package".to_string(),
            recovery: RecoveryStatus::PreviousVersionRestored,
        },
    });
    let first = finalize_at(&paths, request.clone()).expect("first finalize");
    let second = finalize_at(&paths, request).expect("repeat finalize");
    assert!(!first.seeded);
    assert!(!second.seeded);
    assert_eq!(first.errors.len(), 2);
    assert_eq!(second.errors.len(), 2);

    let incidents = store
        .error_incidents(&ErrorScope::new("runtime", "default"), 10)
        .expect("runtime errors");
    assert_eq!(incidents.len(), 2);
    assert!(
        incidents
            .iter()
            .all(|incident| incident.occurrence_count == 2 && incident.revision == 1)
    );
    let inbox_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM strand_inbox", [], |row| row.get(0))
        .expect("inbox count");
    assert_eq!(inbox_count, 500);
}

fn request(terminal: UpgradeTerminal) -> UpgradeFinalizeRequest {
    UpgradeFinalizeRequest {
        protocol_version: 1,
        attempt_id: "upgrade_test".to_string(),
        deb: "/tmp/santi.deb".to_string(),
        terminal,
        readiness: UpgradeReadiness::Ready,
        wake: true,
        soul_id: santi_core::DEFAULT_SOUL_ID.to_string(),
        configured_strand_id: None,
    }
}

fn paths_under(root: &Path) -> RuntimePaths {
    RuntimePaths {
        database_path: root.join("runtime").join("db"),
        runtime_root: root.join("runtime"),
        execution_root: root.join("execution"),
    }
}
