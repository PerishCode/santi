use std::path::Path;

use santi_api::{
    config::RuntimePaths,
    upgrade::{
        RecoveryStatus, UpgradeFailure, UpgradeFinalizeRequest, UpgradeReadiness, UpgradeStage,
        UpgradeTerminal, finalize_at, seed_attempt_handover_at, seed_come_look_at,
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
fn attempt_labels_isolate_rooms() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    SantiStore::open(&paths.database_path).expect("open");

    let outcome = seed_attempt_handover_at(
        &paths,
        santi_core::DEFAULT_SOUL_ID,
        "upgrade_one",
        Some("ss_stale"),
        "come look",
    )
    .expect("seed via attempt label");
    assert!(outcome.warnings.is_empty());
    let retry = seed_attempt_handover_at(
        &paths,
        santi_core::DEFAULT_SOUL_ID,
        "upgrade_one",
        Some("ss_stale"),
        "come look again",
    )
    .expect("repeat seed via attempt label");
    assert_eq!(retry.strand_id, outcome.strand_id);
    let other = seed_attempt_handover_at(
        &paths,
        santi_core::DEFAULT_SOUL_ID,
        "upgrade_two",
        Some("ss_stale"),
        "other attempt",
    )
    .expect("seed other attempt");
    assert_ne!(other.strand_id, outcome.strand_id);

    let store = SantiStore::open(&paths.database_path).expect("reopen");
    let strand = store.strand(&outcome.strand_id).unwrap().expect("strand");
    assert_eq!(
        strand.external_label.as_deref(),
        Some("soul:soul_default:ops:upgrade:upgrade_one")
    );
    let other_strand = store
        .strand(&other.strand_id)
        .unwrap()
        .expect("other strand");
    assert_eq!(
        other_strand.external_label.as_deref(),
        Some("soul:soul_default:ops:upgrade:upgrade_two")
    );
    let started = store
        .try_start_turn(&outcome.strand_id, "strand_send", None)
        .unwrap()
        .expect("turn starts");
    assert_eq!(started.drained_messages[0].content_text, "come look");
    assert_eq!(started.drained_messages[1].content_text, "come look again");
}

#[test]
fn stable_helper_preserves_label() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    SantiStore::open(&paths.database_path).expect("open");

    let outcome = seed_come_look_at(&paths, santi_core::DEFAULT_SOUL_ID, None, "stable wake")
        .expect("seed stable label");
    let store = SantiStore::open(&paths.database_path).expect("reopen");
    let strand = store.strand(&outcome.strand_id).unwrap().expect("strand");
    assert_eq!(
        strand.external_label.as_deref(),
        Some("soul:soul_default:ops")
    );
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
    let seeded = seed_attempt_handover_at(
        &paths,
        santi_core::DEFAULT_SOUL_ID,
        "upgrade_test",
        None,
        "existing wake",
    )
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

#[test]
fn next_attempt_bypasses_exhaustion() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    let first = finalize_at(
        &paths,
        request_for("upgrade_full", UpgradeTerminal::Upgraded),
    )
    .expect("first finalize");
    let first_strand = first.seeded_strand_id.expect("first seeded strand");

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
        SELECT 'inbox_isolation_fixture_' || n, ?1, 'santi_system', '{}',
               NULL, NULL, NULL, 'fixture'
        FROM seq
        "#,
        [&first_strand],
    )
    .expect("fill first attempt room");

    let blocked = finalize_at(
        &paths,
        request_for("upgrade_full", UpgradeTerminal::Upgraded),
    )
    .expect("repeat full attempt");
    assert!(!blocked.seeded);
    assert_eq!(blocked.errors.len(), 1);
    assert_eq!(blocked.errors[0].code, "runtime.upgrade.handover_failed");

    let next = finalize_at(
        &paths,
        request_for("upgrade_next", UpgradeTerminal::Upgraded),
    )
    .expect("next finalize");
    assert!(next.seeded);
    assert!(next.errors.is_empty());
    assert_ne!(
        next.seeded_strand_id.as_deref(),
        Some(first_strand.as_str())
    );

    let store = SantiStore::open(&paths.database_path).expect("open store");
    let handover = store
        .error_incidents(&ErrorScope::new("runtime", "default"), 10)
        .expect("runtime incidents")
        .into_iter()
        .find(|incident| incident.code == "runtime.upgrade.handover_failed")
        .expect("handover incident");
    assert_eq!(handover.status, IncidentStatus::Resolved);
    assert_eq!(
        handover.resolved_by.as_deref(),
        Some("upgrade.handover_succeeded")
    );
}

fn request(terminal: UpgradeTerminal) -> UpgradeFinalizeRequest {
    request_for("upgrade_test", terminal)
}

fn request_for(attempt_id: &str, terminal: UpgradeTerminal) -> UpgradeFinalizeRequest {
    UpgradeFinalizeRequest {
        protocol_version: 1,
        attempt_id: attempt_id.to_string(),
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
