use super::support::*;
use santi_core::{message, turn};

#[test]
fn recovers() {
    let temp = tempfile::tempdir().expect("temp dir");
    let database = temp.path().join("santi.sqlite");
    let store = Store::open(&database).expect("open store");
    let strand = store.weave().expect("create strand");
    store
        .receive(
            &strand.id,
            message::Kind::Text,
            message::Content::text("stop durably"),
            None,
        )
        .expect("receive");
    let started = store
        .tried(&strand.id, "strand_send", None)
        .expect("start")
        .expect("running");
    let turn = started.turn.id;
    drop(store);

    let conn = Connection::open(&database).expect("open sqlite");
    conn.execute(
        "INSERT INTO turn_stops (turn_id, cause, requested_at) VALUES (?1, 'operator', ?2)",
        [&turn, "2026-07-28T00:00:00.000Z"],
    )
    .expect("record stop intent");
    drop(conn);

    let store = Store::open(&database).expect("recover store");
    assert!(
        store
            .complete(santi_core::Completion {
                turn: &turn,
                sequence: None,
                provider: "test",
                model: "test",
                response: None,
            })
            .is_err(),
        "durable stop intent must fence completion"
    );
    assert_eq!(store.reconciled().expect("reconcile turns"), 1);
    let runtime = store
        .snapshot(&strand.id)
        .expect("snapshot")
        .expect("strand");
    let stopped = runtime
        .turns
        .iter()
        .find(|held| held.id == turn)
        .expect("turn");
    assert_eq!(stopped.status, turn::Status::Failed);
    assert_eq!(stopped.error.as_deref(), Some("interrupted by operator"));
    assert!(runtime.errors.is_empty());
    assert!(
        runtime
            .messages
            .iter()
            .any(|message| message.text.contains("interrupted by operator"))
    );
    let request = runtime
        .messages
        .iter()
        .find(|message| message.text == "stop durably")
        .expect("request message");
    assert_eq!(request.message.state, message::State::Fixed);
}
