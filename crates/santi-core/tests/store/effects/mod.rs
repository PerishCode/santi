use super::support::*;

mod more;

struct StartedEffect {
    turn: String,
    effect_id: String,
    inbox: String,
}

fn start_effect(store: &SantiStore) -> StartedEffect {
    let strand = store.create_strand().expect("create strand");
    let inbox = match store
        .enqueue_inbox(
            &strand.id,
            MessageKind::Text,
            MessageContent::text("run an external effect"),
        )
        .expect("enqueue")
    {
        IngestOutcome::Accepted { receipt } => receipt.inbox,
        IngestOutcome::Rejected { .. } => panic!("unexpected rejection"),
    };
    let turn = store
        .try_start_turn(&strand.id, "strand_send", None)
        .expect("start turn")
        .expect("started turn")
        .turn;
    let call = "call_effect".to_string();
    let (_, effect) = store
        .append_effect_call(
            Invocation {
                turn: &turn.id,
                call: &call,
                name: "shell",
                arguments: &json!({"command": "printf external"}),
                provenance: &ToolCallProvenance::default(),
            },
            Some("shell"),
        )
        .expect("append effect intent");
    StartedEffect {
        turn: turn.id,
        effect_id: effect.expect("effect").id,
        inbox,
    }
}

#[test]
fn prepared_failure() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let started = start_effect(&store);

    store
        .fail_turn(&started.turn, "failure before dispatch")
        .expect("fail turn");

    let status = store
        .effect_status(&started.effect_id)
        .expect("query effect")
        .expect("effect");
    assert_eq!(status.effect.state, EffectState::NotDispatched);
    assert_eq!(
        status
            .transitions
            .iter()
            .map(|transition| transition.reason.clone())
            .collect::<Vec<_>>(),
        vec![
            EffectTransitionReason::IntentPersisted,
            EffectTransitionReason::TurnFailedBeforeDispatch,
        ]
    );
    let receipt = store
        .receipt_status(&started.inbox)
        .expect("query receipt")
        .expect("receipt");
    assert_eq!(receipt.state, ReceiptState::TurnFailed);
    assert_eq!(receipt.effects.len(), 1);
    assert_eq!(receipt.effects[0].state, EffectState::NotDispatched);
}

#[test]
fn intent_atomicity() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    let store = SantiStore::open(&db).expect("open store");
    let strand = store.create_strand().expect("create strand");
    store
        .enqueue_inbox(
            &strand.id,
            MessageKind::Text,
            MessageContent::text("run an external effect"),
        )
        .expect("enqueue");
    let turn = store
        .try_start_turn(&strand.id, "strand_send", None)
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
            .append_effect_call(
                Invocation {
                    turn: &turn.id,
                    call: "call_atomic",
                    name: "shell",
                    arguments: &json!({"command": "printf atomic"}),
                    provenance: &ToolCallProvenance::default(),
                },
                Some("shell"),
            )
            .is_err()
    );
    assert!(
        store
            .tool_calls_for_turn(&turn.id)
            .expect("tool calls")
            .is_empty(),
        "the tool call must roll back when its effect intent cannot persist"
    );
}
