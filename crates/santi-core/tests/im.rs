use santi_core::{
    Completion, DEFAULT_SOUL_ID, ImDeliveryMode, IngestOutcome, MessageContent, MessageKind,
    SantiStore,
};

fn store() -> (SantiStore, tempfile::TempDir) {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("db")).expect("open store");
    (store, temp)
}

#[test]
fn participant_is_idempotent() {
    let (store, _temp) = store();
    store.ensure_im_participant("operator", "human").unwrap();
    store.ensure_im_participant("operator", "human").unwrap();
}

#[test]
fn inbox_cursor_orders() {
    let (store, _temp) = store();
    store.ensure_im_participant("alice", "human").unwrap();
    store.ensure_im_participant("bob", "human").unwrap();

    let first = store.enqueue_im_inbox("alice", Some("ss_1"), "hi").unwrap();
    let second = store
        .enqueue_im_inbox("alice", Some("ss_1"), "again")
        .unwrap();
    assert!(second.seq > first.seq);
    store.enqueue_im_inbox("bob", None, "for bob").unwrap();

    let all = store.poll_im_inbox("alice", 0).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].content, "hi");
    assert_eq!(all[1].content, "again");
    assert_eq!(all[0].from_ref.as_deref(), Some("ss_1"));

    let tail = store.poll_im_inbox("alice", first.seq).unwrap();
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].seq, second.seq);
    assert!(store.poll_im_inbox("alice", second.seq).unwrap().is_empty());
}

#[test]
fn participant_resolves_label() {
    let (store, _temp) = store();
    let strand = store
        .find_labeled_strand(DEFAULT_SOUL_ID, "im:alice")
        .unwrap();
    assert_eq!(
        store
            .im_participant_for_strand(&strand.id)
            .unwrap()
            .as_deref(),
        Some("alice")
    );

    let plain = store.create_strand().unwrap();
    assert!(
        store
            .im_participant_for_strand(&plain.id)
            .unwrap()
            .is_none()
    );
}

#[test]
fn turn_reply_deduplicates() {
    let (store, _temp) = store();
    store.ensure_im_participant("operator", "human").unwrap();
    let strand = store
        .find_labeled_strand(DEFAULT_SOUL_ID, "im:operator")
        .unwrap();
    let outcome = store
        .enqueue_inbox(&strand.id, MessageKind::Text, MessageContent::text("hello"))
        .unwrap();
    let IngestOutcome::Accepted { receipt } = outcome else {
        panic!("inbox rejected");
    };
    let turn = store
        .try_start_turn(&strand.id, "strand_send", None)
        .unwrap()
        .expect("turn")
        .turn;

    let (early, inserted) = store
        .enqueue_turn_reply(santi_core::Reply {
            strand: &strand.id,
            turn: &turn.id,
            message: None,
            content: "early",
            mode: ImDeliveryMode::Explicit,
        })
        .unwrap();
    assert!(inserted);
    let (same, inserted) = store
        .enqueue_turn_reply(santi_core::Reply {
            strand: &strand.id,
            turn: &turn.id,
            message: Some("msg_final"),
            content: "final",
            mode: ImDeliveryMode::Automatic,
        })
        .unwrap();
    assert!(!inserted);
    assert_eq!(same.id, early.id);
    assert_eq!(same.content, "early");
    assert_eq!(same.delivery_mode, Some(ImDeliveryMode::Explicit));

    store
        .complete_turn(Completion {
            turn: &turn.id,
            sequence: None,
            provider: "fake",
            model: "fake-model",
            response: None,
        })
        .unwrap();
    let status = store
        .receipt_status(&receipt.inbox_id)
        .unwrap()
        .expect("receipt");
    assert_eq!(status.im_deliveries.len(), 1);
    assert_eq!(status.im_deliveries[0].id, early.id);
    assert_eq!(
        status.im_deliveries[0].delivery_mode,
        ImDeliveryMode::Explicit
    );
}

#[test]
fn v26_history_migrates() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE im_inbox (
            seq INTEGER PRIMARY KEY AUTOINCREMENT,
            id TEXT NOT NULL UNIQUE,
            participant_id TEXT NOT NULL,
            from_ref TEXT,
            content TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        INSERT INTO im_inbox (
            id, participant_id, from_ref, content, created_at
        ) VALUES (
            'imx_legacy', 'operator', 'ss_legacy', 'old reply',
            '2026-07-13T00:00:00.000Z'
        );
        PRAGMA user_version = 26;
        "#,
    )
    .unwrap();
    drop(conn);

    let store = SantiStore::open(&db).expect("migrate v26");
    let entries = store.poll_im_inbox("operator", 0).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].content, "old reply");
    assert_eq!(entries[0].turn_id, None);
    assert_eq!(entries[0].message_id, None);
    assert_eq!(entries[0].delivery_mode, None);
    assert_eq!(
        santi_core::read_schema_version(&db).unwrap(),
        Some(santi_core::SCHEMA_VERSION)
    );
}
