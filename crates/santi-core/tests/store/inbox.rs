use super::support::*;

#[test]
fn drive_coalesces_redrives() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let strand = store.create_strand().expect("create strand");

    // No requests → not behind → no turn.
    assert!(
        store
            .try_start_turn(&strand.id, "strand_send", None)
            .expect("try")
            .is_none()
    );

    // A REQUEST makes the thread behind → starts a turn.
    append_timeline_message(
        &store,
        &strand.id,
        ActorType::System,
        "hi",
        MessageIntake::Request,
    );
    let started = store
        .try_start_turn(&strand.id, "strand_send", None)
        .expect("try")
        .expect("turn started");
    assert_eq!(started.turn.status, santi_core::TurnStatus::Running);
    assert_eq!(started.drained_messages.len(), 1);
    let turn = started.turn;

    // A second request while the turn runs coalesces — no concurrent turn.
    append_timeline_message(
        &store,
        &strand.id,
        ActorType::System,
        "and again",
        MessageIntake::Request,
    );
    assert!(
        store
            .try_start_turn(&strand.id, "strand_send", None)
            .expect("try")
            .is_none(),
        "a running turn must block a second concurrent turn"
    );

    // After the turn completes, the request that arrived during it is past the
    // turn's start → behind again → drive the next turn.
    store
        .complete_turn(&turn.id, None, "fake", None)
        .expect("complete");
    assert!(
        store
            .try_start_turn(&strand.id, "strand_send", None)
            .expect("try")
            .is_some(),
        "accumulated request should drive the next turn at completion"
    );
}

#[test]
fn drain_commits_pending() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let strand = store.create_strand().expect("create strand");

    // Multiple adaptors can enqueue concurrently before the driver ever runs;
    // the NEXT drive drains everything present into ONE turn, in arrival order.
    for text in ["first", "second", "third"] {
        store
            .enqueue_inbox(&strand.id, MessageKind::Text, MessageContent::text(text))
            .expect("enqueue");
    }
    let started = store
        .try_start_turn(&strand.id, "strand_send", None)
        .expect("try")
        .expect("turn started");
    assert_eq!(started.drained_messages.len(), 3);
    assert_eq!(started.drained_messages[0].content_text, "first");
    assert_eq!(started.drained_messages[1].content_text, "second");
    assert_eq!(started.drained_messages[2].content_text, "third");
    for (index, message) in started.drained_messages.iter().enumerate() {
        assert_eq!(message.relation.strand_seq, (index + 1) as i64);
    }

    // The inbox is now empty — nothing left to drain, no new turn.
    assert!(
        store
            .try_start_turn(&strand.id, "strand_send", None)
            .expect("try")
            .is_none()
    );
}

#[test]
fn drain_records_provenance() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    let store = SantiStore::open(&db).expect("open store");
    let strand = store.create_strand().expect("create strand");

    store
        .enqueue_inbox_with_source(
            &strand.id,
            MessageKind::Text,
            MessageContent::text("needs provenance"),
            Some(
                santi_core::InboxSource::new("test")
                    .with_ref("caller-1")
                    .with_metadata(serde_json::json!({ "adaptor": "fake" })),
            ),
        )
        .expect("enqueue with source");

    let conn = Connection::open(&db).expect("open sqlite");
    let (inbox_id, enqueued_at): (String, String) = conn
        .query_row(
            "SELECT id, created_at FROM strand_inbox WHERE strand_id = ?1",
            [&strand.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("inbox row");
    drop(conn);

    let started = store
        .try_start_turn(&strand.id, "strand_send", None)
        .expect("try")
        .expect("turn started");
    assert_eq!(started.drained_messages.len(), 1);
    let drained = &started.drained_messages[0];

    let runtime = store
        .runtime_snapshot(&strand.id)
        .expect("runtime snapshot")
        .expect("strand runtime");
    assert_eq!(runtime.message_events.len(), 1);
    let event = &runtime.message_events[0];
    assert_eq!(event.action, "insert");
    assert_eq!(event.message_id, drained.message.id);
    assert_eq!(event.created_at, drained.message.created_at);

    let payload = &event.payload;
    assert_eq!(payload["kind"], "inbox_drain");
    assert_eq!(payload["inbox_id"], inbox_id);
    assert_eq!(payload["enqueued_at"], enqueued_at);
    assert_eq!(payload["drained_at"], drained.message.created_at);
    assert_eq!(payload["committing_turn_id"], started.turn.id);
    assert_eq!(payload["message_id"], drained.message.id);
    assert_eq!(payload["strand_seq"], drained.relation.strand_seq);
    assert_eq!(payload["source"]["type"], "test");
    assert_eq!(payload["source"]["ref"], "caller-1");
    assert_eq!(payload["source"]["metadata"]["adaptor"], "fake");

    let input = store.assembly_input(&strand.id).expect("assembly input");
    assert_eq!(input.len(), 1);
    assert_text(&input[0], "user", "needs provenance");
}

#[test]
fn inbox_gate_rejects() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let strand = store.create_strand().expect("create strand");

    // Never drained (no try_start_turn call), so every enqueue adds to the
    // undrained count — eventually the gate must start rejecting rather than
    // growing without bound.
    let mut rejected = false;
    for _ in 0..600 {
        match store
            .enqueue_inbox(&strand.id, MessageKind::Text, MessageContent::text("x"))
            .expect("enqueue")
        {
            IngestOutcome::Accepted { .. } => {}
            IngestOutcome::Rejected { error } => {
                assert_eq!(error.code, "runtime.inbox.capacity_exceeded");
                assert!(error.message.contains("inbox is full"), "got: {error}");
                rejected = true;
                break;
            }
        }
    }
    assert!(rejected, "gate never rejected after 600 enqueues");
}

#[test]
fn records_do_not_drive() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let strand = store.create_strand().expect("create strand");

    // A RECORD (the soul's own output / a failure notice) is not a request and
    // must not wake the soul.
    append_timeline_message(
        &store,
        &strand.id,
        ActorType::Soul,
        "a note to self",
        MessageIntake::Record,
    );
    assert!(
        store
            .try_start_turn(&strand.id, "strand_send", None)
            .expect("try")
            .is_none(),
        "record messages must not drive a turn"
    );
    assert!(
        store
            .strands_with_pending_requests()
            .expect("scan")
            .is_empty()
    );
}

#[test]
fn boot_reconciles_once() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let strand = store.create_strand().expect("create strand");
    append_timeline_message(
        &store,
        &strand.id,
        ActorType::System,
        "do a thing",
        MessageIntake::Request,
    );
    // A turn starts and then the process "crashes" (turn left running).
    store
        .try_start_turn(&strand.id, "strand_send", None)
        .expect("try")
        .expect("turn started");
    // Before recovery it is the only pending-driver; reconcile interrupts it.
    assert_eq!(store.reconcile_orphaned_turns().expect("reconcile"), 1);
    // The interrupted turn counts as "attempted" → the request is NOT retried.
    assert!(
        store
            .try_start_turn(&strand.id, "strand_send", None)
            .expect("try")
            .is_none(),
        "an interrupted turn must not auto-retry its request"
    );
    // But a genuinely new request drives a fresh turn (liveness).
    append_timeline_message(
        &store,
        &strand.id,
        ActorType::System,
        "a new thing",
        MessageIntake::Request,
    );
    assert!(
        store
            .strands_with_pending_requests()
            .expect("scan")
            .iter()
            .any(|id| id == &strand.id)
    );
    assert!(
        store
            .try_start_turn(&strand.id, "strand_send", None)
            .expect("try")
            .is_some()
    );
}
