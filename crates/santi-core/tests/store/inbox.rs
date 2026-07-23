use super::support::*;
use santi_core::{ingest, message};

#[test]
fn drive_coalesces_redrives() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let strand = store.create_strand().expect("create strand");

    assert!(
        store
            .try_start_turn(&strand.id, "strand_send", None)
            .expect("try")
            .is_none()
    );

    append_timeline_message(Line {
        store: &store,
        strand: &strand.id,
        actor: message::Role::System,
        text: "hi",
        intake: message::Intake::Request,
    });
    let started = store
        .try_start_turn(&strand.id, "strand_send", None)
        .expect("try")
        .expect("turn started");
    assert_eq!(started.turn.status, santi_core::turn::Status::Running);
    assert_eq!(started.drained_messages.len(), 1);
    let turn = started.turn;

    append_timeline_message(Line {
        store: &store,
        strand: &strand.id,
        actor: message::Role::System,
        text: "and again",
        intake: message::Intake::Request,
    });
    assert!(
        store
            .try_start_turn(&strand.id, "strand_send", None)
            .expect("try")
            .is_none(),
        "a running turn must block a second concurrent turn"
    );

    store
        .complete(Completion {
            turn: &turn.id,
            sequence: None,
            provider: "fake",
            model: "fake-model",
            response: None,
        })
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

    for text in ["first", "second", "third"] {
        store
            .enqueue_inbox(
                &strand.id,
                message::Kind::Text,
                message::Content::text(text),
            )
            .expect("enqueue");
    }
    let started = store
        .try_start_turn(&strand.id, "strand_send", None)
        .expect("try")
        .expect("turn started");
    assert_eq!(started.drained_messages.len(), 3);
    assert_eq!(started.drained_messages[0].text, "first");
    assert_eq!(started.drained_messages[1].text, "second");
    assert_eq!(started.drained_messages[2].text, "third");
    for (index, message) in started.drained_messages.iter().enumerate() {
        assert_eq!(message.relation.seq, (index + 1) as i64);
    }

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
            message::Kind::Text,
            message::Content::text("needs provenance"),
            Some(
                santi_core::ingest::Source::new("test")
                    .with_ref("caller-1")
                    .with_metadata(serde_json::json!({ "adaptor": "fake" })),
            ),
        )
        .expect("enqueue with source");

    let conn = Connection::open(&db).expect("open sqlite");
    let (inbox, queued): (String, String) = conn
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
    assert_eq!(runtime.events.len(), 1);
    let event = &runtime.events[0];
    assert_eq!(event.action, "insert");
    assert_eq!(event.message, drained.message.id);
    assert_eq!(event.created, drained.message.created);

    let payload = &event.payload;
    assert_eq!(payload["kind"], "inbox_drain");
    assert_eq!(payload["inbox"], inbox);
    assert_eq!(payload["queued"], queued);
    assert_eq!(payload["drained_at"], drained.message.created);
    assert_eq!(payload["committing_turn_id"], started.turn.id);
    assert_eq!(payload["message"], drained.message.id);
    assert_eq!(payload["seq"], drained.relation.seq);
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
    let db = temp.path().join("santi.sqlite");
    let store = SantiStore::open(&db).expect("open store");
    let strand = store.create_strand().expect("create strand");

    let mut conn = Connection::open(&db).expect("open sqlite");
    let tx = conn.transaction().expect("begin seed transaction");
    {
        let mut insert_inbox = tx
            .prepare(
                "INSERT INTO strand_inbox \
                 (id, strand_id, message_kind, content, created_at) \
                 VALUES (?1, ?2, 'text', '{}', '2026-07-13T00:00:00Z')",
            )
            .expect("prepare inbox seed");
        let mut insert_receipt = tx
            .prepare(
                "INSERT INTO inbox_receipts \
                 (id, strand_id, state, accepted_at, updated_at) \
                 VALUES (?1, ?2, 'accepted', \
                         '2026-07-13T00:00:00Z', '2026-07-13T00:00:00Z')",
            )
            .expect("prepare receipt seed");
        let mut insert_transition = tx
            .prepare(
                "INSERT INTO receipt_transitions \
                 (id, inbox_id, sequence, state, occurred_at) \
                 VALUES (?1, ?2, 1, 'accepted', '2026-07-13T00:00:00Z')",
            )
            .expect("prepare receipt transition seed");
        for index in 0..500 {
            let inbox = format!("inbox_gate_seed_{index}");
            insert_inbox
                .execute(rusqlite::params![&inbox, &strand.id])
                .expect("seed inbox row");
            insert_receipt
                .execute(rusqlite::params![&inbox, &strand.id])
                .expect("seed inbox receipt");
            insert_transition
                .execute(rusqlite::params![
                    format!("receipt_transition_seed_{index}"),
                    &inbox
                ])
                .expect("seed receipt transition");
        }
    }
    tx.commit().expect("commit inbox seed");
    drop(conn);

    let outcome = store
        .enqueue_inbox(&strand.id, message::Kind::Text, message::Content::text("x"))
        .expect("enqueue at gate");
    let ingest::Outcome::Rejected { error } = outcome else {
        panic!("gate accepted an enqueue after 500 pending rows");
    };
    assert_eq!(error.code, "runtime.inbox.capacity_exceeded");
    assert!(error.message.contains("inbox is full"), "got: {error}");
}

#[test]
fn records_do_not_drive() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let strand = store.create_strand().expect("create strand");

    append_timeline_message(Line {
        store: &store,
        strand: &strand.id,
        actor: message::Role::Soul,
        text: "a note to self",
        intake: message::Intake::Record,
    });
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
    append_timeline_message(Line {
        store: &store,
        strand: &strand.id,
        actor: message::Role::System,
        text: "do a thing",
        intake: message::Intake::Request,
    });
    store
        .try_start_turn(&strand.id, "strand_send", None)
        .expect("try")
        .expect("turn started");
    assert_eq!(store.reconcile_orphaned_turns().expect("reconcile"), 1);
    assert!(
        store
            .try_start_turn(&strand.id, "strand_send", None)
            .expect("try")
            .is_none(),
        "an interrupted turn must not auto-retry its request"
    );
    append_timeline_message(Line {
        store: &store,
        strand: &strand.id,
        actor: message::Role::System,
        text: "a new thing",
        intake: message::Intake::Request,
    });
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
