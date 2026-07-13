use super::support::*;

#[test]
fn v24_backfills_receipts() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    let store = SantiStore::open(&db).expect("open store");
    let strand = store.create_strand().expect("create strand");
    let completed_id = match store
        .enqueue_inbox(
            &strand.id,
            MessageKind::Text,
            MessageContent::text("completed obligation"),
        )
        .expect("enqueue completed")
    {
        IngestOutcome::Accepted { receipt } => receipt.inbox_id,
        IngestOutcome::Rejected { .. } => panic!("unexpected rejection"),
    };
    let turn = store
        .try_start_turn(&strand.id, "strand_send", None)
        .expect("start")
        .expect("started")
        .turn;
    store
        .complete_turn(&turn.id, None, "fake", "fake-model", None)
        .expect("complete");
    let pending_id = match store
        .enqueue_inbox(
            &strand.id,
            MessageKind::Text,
            MessageContent::text("pending obligation"),
        )
        .expect("enqueue pending")
    {
        IngestOutcome::Accepted { receipt } => receipt.inbox_id,
        IngestOutcome::Rejected { .. } => panic!("unexpected rejection"),
    };
    drop(store);

    let conn = Connection::open(&db).expect("open sqlite");
    conn.execute_batch(
        r#"
        DROP TABLE receipt_transitions;
        DROP TABLE inbox_receipts;
        PRAGMA user_version = 24;
        "#,
    )
    .expect("downgrade shape");
    drop(conn);

    let store = SantiStore::open(&db).expect("migrate v24 to v25");
    let completed = store
        .receipt_status(&completed_id)
        .expect("completed query")
        .expect("completed receipt");
    assert_eq!(completed.state, santi_core::ReceiptState::Completed);
    assert_eq!(completed.transitions.len(), 3);
    let completed_source = completed.transitions[0]
        .reconstructed_from
        .as_deref()
        .expect("completed reconstruction source");
    assert!(completed_source.starts_with("v24:message_event:"));
    assert!(
        completed
            .transitions
            .iter()
            .all(|transition| transition.reconstructed_from.as_deref() == Some(completed_source))
    );
    let pending = store
        .receipt_status(&pending_id)
        .expect("pending query")
        .expect("pending receipt");
    assert_eq!(pending.state, santi_core::ReceiptState::Accepted);
    assert_eq!(pending.transitions.len(), 1);
    assert_eq!(
        pending.transitions[0].reconstructed_from.as_deref(),
        Some(format!("v24:strand_inbox:{pending_id}").as_str())
    );
}
