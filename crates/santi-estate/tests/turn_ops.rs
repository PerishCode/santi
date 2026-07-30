use santi_estate::{OutboxDraft, Store, StrandDraft, TurnDraft};
use santi_model::{event, turn};

const SUDO: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const FIRST: &str = "2026-07-28T00:00:00.000Z";
const LATER: &str = "2026-07-28T00:01:00.000Z";
const LAST: &str = "2026-07-28T00:02:00.000Z";

#[tokio::test]
async fn contracts() {
    let temp = tempfile::tempdir().expect("temp");
    let path = temp.path().join("estate.sqlite");
    let store = Store::bootstrap(&path, SUDO).await.expect("open");
    store.seed("soul_test", FIRST).await.expect("seed");
    let worker = store
        .create_strand(StrandDraft {
            tag: "strand_worker",
            soul: "soul_test",
            label: Some("worker/thread"),
            parent: None,
            fork: None,
            created: FIRST,
        })
        .await
        .expect("worker");
    let other = store
        .create_strand(StrandDraft {
            tag: "strand_other",
            soul: "soul_test",
            label: Some("other/thread"),
            parent: None,
            fork: None,
            created: FIRST,
        })
        .await
        .expect("other");

    create_turn(&store, "turn_one", &worker.id).await;
    store
        .complete_turn("turn_one", 0, FIRST)
        .await
        .expect("complete one");
    create_turn(&store, "turn_two", &other.id).await;
    store
        .complete_turn("turn_two", 0, LATER)
        .await
        .expect("complete two");
    create_turn(&store, "turn_three", &worker.id).await;
    store
        .complete_turn("turn_three", 0, LAST)
        .await
        .expect("complete three");

    let one = event(Draft {
        id: "event_one",
        turn: "turn_one",
        strand: &worker.id,
        label: "worker/thread",
        completed: FIRST,
    });
    let two = event(Draft {
        id: "event_two",
        turn: "turn_two",
        strand: &other.id,
        label: "other/thread",
        completed: LATER,
    });
    let three = event(Draft {
        id: "event_three",
        turn: "turn_three",
        strand: &worker.id,
        label: "worker/thread",
        completed: LAST,
    });
    for event in [&one, &two, &three] {
        store
            .queue_outbox(OutboxDraft {
                stream: "turns",
                event,
            })
            .await
            .expect("queue");
    }
    store
        .queue_outbox(OutboxDraft {
            stream: "turns",
            event: &one,
        })
        .await
        .expect("replay queue");
    let conflict = event(Draft {
        id: "event_conflict",
        turn: "turn_one",
        strand: &worker.id,
        label: "worker/thread",
        completed: FIRST,
    });
    assert!(
        store
            .queue_outbox(OutboxDraft {
                stream: "turns",
                event: &conflict,
            })
            .await
            .is_err()
    );

    let first = store.outbox("turns", 0, "worker/", 1).await.expect("first");
    assert_eq!(first.cursor, 1);
    assert_eq!(first.events[0].id, "event_one");
    let rest = store.outbox("turns", 1, "worker/", 1).await.expect("rest");
    assert_eq!(rest.cursor, 3);
    assert_eq!(rest.events[0].id, "event_three");
    let empty = store
        .outbox("turns", 0, "missing/", 10)
        .await
        .expect("empty");
    assert_eq!(empty.cursor, 3);
    assert!(empty.events.is_empty());

    create_turn(&store, "turn_stop", &worker.id).await;
    let stop = store
        .request_stop("turn_stop", turn::Cause::Operator, FIRST)
        .await
        .expect("request")
        .expect("turn");
    assert!(stop.accepted);
    assert_eq!(stop.cause, Some(turn::Cause::Operator));
    let repeated = store
        .request_stop("turn_stop", turn::Cause::Shutdown, LATER)
        .await
        .expect("repeat")
        .expect("turn");
    assert_eq!(repeated.cause, Some(turn::Cause::Operator));
    assert_eq!(repeated.requested.as_deref(), Some(FIRST));
    assert!(store.settle_stop("turn_stop", LATER).await.is_err());
    assert!(store.complete_turn("turn_stop", 0, LATER).await.is_err());
    store
        .fail_turn("turn_stop", "interrupted by operator", LATER)
        .await
        .expect("fail");
    let settled = store.settle_stop("turn_stop", LATER).await.expect("settle");
    assert_eq!(settled.settled.as_deref(), Some(LATER));
    let settled = store
        .settle_stop("turn_stop", LAST)
        .await
        .expect("settle twice");
    assert_eq!(settled.settled.as_deref(), Some(LATER));

    let terminal = store
        .request_stop("turn_one", turn::Cause::Operator, LAST)
        .await
        .expect("terminal")
        .expect("turn");
    assert!(!terminal.accepted);
    assert!(
        store
            .request_stop("turn_missing", turn::Cause::Operator, LAST)
            .await
            .expect("missing")
            .is_none()
    );

    drop(store);
    let store = Store::open(path).await.expect("open again");
    assert_eq!(
        store
            .stop("turn_stop")
            .await
            .expect("stop")
            .expect("turn")
            .settled
            .as_deref(),
        Some(LATER)
    );
    assert_eq!(
        store
            .outbox("turns", 0, "", 10)
            .await
            .expect("outbox")
            .events
            .len(),
        3
    );
}

async fn create_turn(store: &Store, tag: &str, strand: &str) {
    store
        .create_turn(TurnDraft {
            tag,
            strand,
            trigger: turn::Trigger::System,
            source: None,
            from: 0,
            created: FIRST,
        })
        .await
        .expect("turn");
}

struct Draft<'a> {
    id: &'a str,
    turn: &'a str,
    strand: &'a str,
    label: &'a str,
    completed: &'a str,
}

fn event(draft: Draft<'_>) -> event::Event {
    event::Event {
        id: draft.id.to_string(),
        strand: draft.strand.to_string(),
        turn: draft.turn.to_string(),
        label: draft.label.to_string(),
        text: format!("completed {}", draft.turn),
        completed: draft.completed.to_string(),
    }
}
