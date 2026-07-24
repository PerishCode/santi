use super::*;
use santi_core::{effect, tool};

#[test]
fn restart_ambiguity() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = Store::open(temp.path().join("santi.sqlite")).expect("open store");
    let (ledger, context) = observed();
    let _entered = context.enter();
    let started = start_effect(&store);
    store
        .dispatch(&started.effect)
        .expect("open dispatch window");
    let (_, prepared) = store
        .charge(
            Invocation {
                turn: &started.turn,
                call: "call_still_prepared",
                name: "shell",
                arguments: &json!({"command": "printf prepared"}),
                provenance: &tool::Provenance::default(),
            },
            Some("shell"),
        )
        .expect("append second effect");
    let prepared_id = prepared.expect("prepared effect").id;

    assert_eq!(store.reconciled().expect("reconcile"), 1);
    let status = store
        .effect(&started.effect)
        .expect("query effect")
        .expect("effect");
    assert_eq!(status.effect.state, effect::State::Unknown);
    assert_eq!(
        reasons(&ledger, &started.effect),
        vec![
            "intent_persisted",
            "dispatch_window_opened",
            "restart_during_dispatch",
        ]
    );
    let prepared = store
        .effect(&prepared_id)
        .expect("query prepared effect")
        .expect("prepared effect");
    assert_eq!(
        prepared.effect.state,
        effect::State::Settled(effect::Outcome::NotApplied)
    );
    assert_eq!(
        reasons(&ledger, &prepared_id).last().map(String::as_str),
        Some("restart_before_dispatch")
    );
    assert!(
        store
            .replied(&started.turn)
            .expect("tool results before resolution")
            .is_empty()
    );

    let resolved = store
        .settle(
            &started.effect,
            effect::Outcome::NotApplied,
            "operator checked the target system",
        )
        .expect("resolve")
        .expect("resolved effect");
    assert_eq!(
        resolved.effect.state,
        effect::State::Settled(effect::Outcome::NotApplied)
    );
    assert_eq!(
        reasons(&ledger, &started.effect).last().map(String::as_str),
        Some("operator_resolved")
    );
    let resolution = ledger
        .query(|record| {
            record.name == "effect.shift"
                && record.says("effect", &started.effect)
                && record.says("reason", "operator_resolved")
        })
        .pop()
        .expect("resolution record");
    assert!(resolution.says("evidence", "operator checked the target system"));
    assert!(resolution.says("state", "settled_not_applied"));
    assert!(
        store
            .replied(&started.turn)
            .expect("tool results after resolution")
            .is_empty(),
        "resolution records evidence; it must not dispatch or fabricate a tool result"
    );
    assert!(
        store
            .settle(&started.effect, effect::Outcome::Applied, "second guess",)
            .is_err(),
        "a settled operator resolution is immutable"
    );
}

#[test]
fn archived_trail() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = Store::open(temp.path().join("santi.sqlite")).expect("open store");
    let context = plumb::context::Context::root().with(store.sink());
    let _entered = context.enter();
    let started = start_effect(&store);
    store
        .dispatch(&started.effect)
        .expect("open dispatch window");

    let records = (0..100)
        .find_map(|_| {
            let records = store.trail("effect", &started.effect).expect("query trail");
            if records.len() >= 2 {
                return Some(records);
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
            None
        })
        .expect("archived trace records");
    assert_eq!(records[0].name, "effect.shift");
    assert!(
        records
            .iter()
            .flat_map(|record| record.tags.iter())
            .any(|tag| tag.key == "reason" && tag.value == "dispatch_window_opened")
    );
}
