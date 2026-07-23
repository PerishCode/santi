use super::*;

#[test]
fn full_handover_is_idempotent() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    let seeded = paths
        .seed_attempt_handover(santi_core::GENESIS, "upgrade_test", None, "existing wake")
        .expect("initial seed");
    let store = Store::open(&paths.database).expect("open store");
    let conn = rusqlite::Connection::open(&paths.database).expect("open sqlite");
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
        [&seeded.strand],
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
        .incidents(&santi_core::Scope::new("runtime", "default"), 10)
        .expect("runtime errors");
    assert_eq!(incidents.len(), 2);
    assert!(
        incidents
            .iter()
            .all(|incident| incident.occurrences == 2 && incident.revision == 1)
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

    let conn = rusqlite::Connection::open(&paths.database).expect("open sqlite");
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

    let store = Store::open(&paths.database).expect("open store");
    let handover = store
        .incidents(&santi_core::Scope::new("runtime", "default"), 10)
        .expect("runtime incidents")
        .into_iter()
        .find(|incident| incident.code == "runtime.upgrade.handover_failed")
        .expect("handover incident");
    assert_eq!(handover.status, santi_core::Status::Resolved);
    assert_eq!(
        handover.resolution.as_ref().unwrap().by.as_deref(),
        Some("upgrade.handover_succeeded")
    );
}
