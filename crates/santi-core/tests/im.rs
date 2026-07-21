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
    let deliveries = store.im_deliveries_for_receipt(&receipt.inbox_id).unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].id, early.id);
    assert_eq!(deliveries[0].delivery_mode, ImDeliveryMode::Explicit);
}
