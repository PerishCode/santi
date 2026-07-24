use super::support::*;
use plumb::context::Context;
use plumb::trace::{Ledger, Sink};
use santi_core::{effect, ingest, message, receipt, tool};
use std::sync::Arc;

mod more;

fn observed() -> (Arc<Ledger>, Context) {
    let ledger = Arc::new(Ledger::default());
    let context = Context::root().with(Sink::from(ledger.clone()));
    (ledger, context)
}

fn reasons(ledger: &Ledger, effect: &str) -> Vec<String> {
    ledger
        .query(|record| record.name == "effect.shift" && record.says("effect", effect))
        .iter()
        .map(|record| {
            record
                .tags
                .iter()
                .find(|(key, _)| key == "reason")
                .map(|(_, value)| value.clone())
                .expect("shift carries a reason tag")
        })
        .collect()
}

struct StartedEffect {
    turn: String,
    effect: String,
    inbox: String,
}

fn start_effect(store: &Store) -> StartedEffect {
    let strand = store.weave().expect("create strand");
    let inbox = match store
        .receive(
            &strand.id,
            message::Kind::Text,
            message::Content::text("run an external effect"),
            None,
        )
        .expect("enqueue")
    {
        ingest::Outcome::Accepted { receipt } => receipt.inbox,
        ingest::Outcome::Rejected { .. } => panic!("unexpected rejection"),
    };
    let turn = store
        .tried(&strand.id, "strand_send", None)
        .expect("start turn")
        .expect("started turn")
        .turn;
    let call = "call_effect".to_string();
    let (_, effect) = store
        .charge(
            Invocation {
                turn: &turn.id,
                call: &call,
                name: "shell",
                arguments: &json!({"command": "printf external"}),
                provenance: &tool::Provenance::default(),
            },
            Some("shell"),
        )
        .expect("append effect intent");
    StartedEffect {
        turn: turn.id,
        effect: effect.expect("effect").id,
        inbox,
    }
}

#[test]
fn prepared_failure() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = Store::open(temp.path().join("santi.sqlite")).expect("open store");
    let (ledger, context) = observed();
    let _entered = context.enter();
    let started = start_effect(&store);

    store
        .fail(&started.turn, "failure before dispatch")
        .expect("fail turn");

    let status = store
        .effect(&started.effect)
        .expect("query effect")
        .expect("effect");
    assert_eq!(
        status.effect.state,
        effect::State::Settled(effect::Outcome::NotApplied)
    );
    assert_eq!(
        reasons(&ledger, &started.effect),
        vec!["intent_persisted", "turn_failed_before_dispatch"]
    );
    let receipt = store
        .receipt(&started.inbox)
        .expect("query receipt")
        .expect("receipt");
    assert_eq!(receipt.state, receipt::State::Failed);
    assert_eq!(receipt.effects.len(), 1);
    assert_eq!(
        receipt.effects[0].state,
        effect::State::Settled(effect::Outcome::NotApplied)
    );
}

#[test]
fn intent_atomicity() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    let store = Store::open(&db).expect("open store");
    let strand = store.weave().expect("create strand");
    store
        .receive(
            &strand.id,
            message::Kind::Text,
            message::Content::text("run an external effect"),
            None,
        )
        .expect("enqueue");
    let turn = store
        .tried(&strand.id, "strand_send", None)
        .expect("start turn")
        .expect("started turn")
        .turn;
    let conn = Connection::open(&db).expect("open sqlite");
    conn.execute_batch(
        r#"
        CREATE TRIGGER reject_effect_intent
        BEFORE INSERT ON strand_effects
        BEGIN
          SELECT RAISE(ABORT, 'forced effect intent failure');
        END;
        "#,
    )
    .expect("install trigger");

    assert!(
        store
            .charge(
                Invocation {
                    turn: &turn.id,
                    call: "call_atomic",
                    name: "shell",
                    arguments: &json!({"command": "printf atomic"}),
                    provenance: &tool::Provenance::default(),
                },
                Some("shell"),
            )
            .is_err()
    );
    assert!(
        store.called(&turn.id).expect("tool calls").is_empty(),
        "the tool call must roll back when its effect intent cannot persist"
    );
}
