use santi_estate::{EffectDraft, InboxDraft, InterruptionDraft, Store, StrandDraft, TurnDraft};
use santi_model::{effect, message, receipt, turn};

const FIRST: &str = "2026-07-28T00:00:00.000Z";
const LATER: &str = "2026-07-28T00:01:00.000Z";
const LAST: &str = "2026-07-28T00:02:00.000Z";

#[tokio::test]
async fn terminal_ceremonies() {
    let temp = tempfile::tempdir().expect("temp");
    let path = temp.path().join("estate.sqlite");
    let store = Store::open(&path).await.expect("open");
    store.seed("soul_test", FIRST).await.expect("seed");

    running(&store, "interrupt").await;
    store
        .request_stop("turn_interrupt", turn::Cause::Operator, FIRST)
        .await
        .expect("request stop");
    let interrupted = store
        .interrupt_turn(InterruptionDraft {
            turn: "turn_interrupt",
            cause: turn::Cause::Shutdown,
            actor: "santi",
            occurred: LATER,
        })
        .await
        .expect("interrupt");
    assert_eq!(interrupted.stop.cause, Some(turn::Cause::Operator));
    assert_eq!(interrupted.stop.requested.as_deref(), Some(FIRST));
    assert_eq!(interrupted.stop.settled.as_deref(), Some(LATER));
    assert_eq!(interrupted.stop.turn.status, turn::Status::Failed);
    assert_eq!(
        interrupted.stop.turn.error.as_deref(),
        Some("interrupted by operator")
    );
    assert!(
        interrupted
            .notice
            .as_ref()
            .is_some_and(|notice| notice.text.contains("interrupted by operator"))
    );
    assert_effect(
        &store,
        "effect_interrupt",
        effect::State::Settled(effect::Outcome::NotApplied),
    )
    .await;
    assert_receipt(&store, "inbox_interrupt", false).await;

    let repeated = store
        .interrupt_turn(InterruptionDraft {
            turn: "turn_interrupt",
            cause: turn::Cause::Shutdown,
            actor: "santi",
            occurred: LAST,
        })
        .await
        .expect("repeat interrupt");
    assert!(repeated.notice.is_none());
    assert_eq!(repeated.stop.settled.as_deref(), Some(LATER));
    assert_eq!(
        store
            .messages("strand_interrupt")
            .await
            .expect("interrupt messages")
            .len(),
        1
    );
    store
        .create_turn(TurnDraft {
            tag: "turn_completed",
            strand: "strand_interrupt",
            trigger: turn::Trigger::System,
            source: None,
            from: 0,
            created: LATER,
        })
        .await
        .expect("completed turn");
    store
        .complete_turn("turn_completed", 1, LATER)
        .await
        .expect("complete");
    let completed = store
        .interrupt_turn(InterruptionDraft {
            turn: "turn_completed",
            cause: turn::Cause::Operator,
            actor: "santi",
            occurred: LAST,
        })
        .await
        .expect("interrupt completed");
    assert!(!completed.stop.accepted);
    assert!(completed.notice.is_none());

    running(&store, "restart").await;
    store
        .dispatch_effect("effect_restart", FIRST)
        .await
        .expect("dispatch");
    running(&store, "stopped").await;
    store
        .request_stop("turn_stopped", turn::Cause::Shutdown, FIRST)
        .await
        .expect("request restart stop");

    assert_eq!(
        store.recover_turns("santi", LATER).await.expect("recover"),
        2
    );
    let restarted = store
        .turn("turn_restart")
        .await
        .expect("restart turn")
        .expect("turn");
    assert_eq!(restarted.status, turn::Status::Failed);
    assert_eq!(restarted.error.as_deref(), Some("interrupted by restart"));
    assert!(
        !store
            .stop("turn_restart")
            .await
            .expect("restart stop")
            .expect("turn")
            .accepted
    );
    assert_effect(&store, "effect_restart", effect::State::Unknown).await;
    assert_receipt(&store, "inbox_restart", true).await;
    let incident = store
        .incident("runtime.turn.failed:strand:strand_restart")
        .await
        .expect("incident")
        .expect("active");
    assert_eq!(incident.latest.context["turn"], "turn_restart");

    let stopped = store
        .stop("turn_stopped")
        .await
        .expect("stopped")
        .expect("turn");
    assert_eq!(stopped.turn.status, turn::Status::Failed);
    assert_eq!(stopped.cause, Some(turn::Cause::Shutdown));
    assert_eq!(stopped.settled.as_deref(), Some(LATER));
    assert_receipt(&store, "inbox_stopped", false).await;
    assert_eq!(
        store
            .messages("strand_stopped")
            .await
            .expect("stopped messages")
            .len(),
        1
    );
    assert_eq!(store.pending_errors(10).await.expect("errors").len(), 1);
    assert_eq!(
        store
            .recover_turns("santi", LAST)
            .await
            .expect("recover twice"),
        0
    );

    drop(store);
    let store = Store::open(path).await.expect("open again");
    assert_eq!(store.running().await.expect("running"), 0);
    assert_eq!(
        store
            .stop("turn_interrupt")
            .await
            .expect("stop")
            .expect("turn")
            .settled
            .as_deref(),
        Some(LATER)
    );
    assert!(
        store
            .incident("runtime.turn.failed:strand:strand_restart")
            .await
            .expect("incident again")
            .is_some()
    );
}

async fn running(store: &Store, suffix: &str) {
    let strand = format!("strand_{suffix}");
    let turn = format!("turn_{suffix}");
    let inbox = format!("inbox_{suffix}");
    let effect = format!("effect_{suffix}");
    store
        .create_strand(StrandDraft {
            tag: &strand,
            soul: "soul_test",
            label: None,
            parent: None,
            fork: None,
            created: FIRST,
        })
        .await
        .expect("strand");
    store
        .accept_inbox(
            InboxDraft {
                tag: &inbox,
                strand: &strand,
                kind: message::Kind::Text,
                content: &message::Content::text("drive"),
                source: None,
                created: FIRST,
            },
            10,
        )
        .await
        .expect("inbox");
    store
        .create_turn(TurnDraft {
            tag: &turn,
            strand: &strand,
            trigger: turn::Trigger::System,
            source: None,
            from: 0,
            created: FIRST,
        })
        .await
        .expect("turn");
    store
        .advance_receipt(
            &inbox,
            receipt::State::Driving,
            Some(&turn),
            None,
            None,
            FIRST,
        )
        .await
        .expect("drive receipt");
    store
        .prepare_effect(EffectDraft {
            tag: &effect,
            turn: &turn,
            call: None,
            kind: "test",
            metadata: None,
            created: FIRST,
        })
        .await
        .expect("effect");
}

async fn assert_effect(store: &Store, tag: &str, state: effect::State) {
    assert_eq!(
        store
            .effect(tag)
            .await
            .expect("effect")
            .expect("status")
            .effect
            .state,
        state
    );
}

async fn assert_receipt(store: &Store, tag: &str, incident: bool) {
    let receipt = store.receipt(tag).await.expect("receipt").expect("status");
    assert_eq!(receipt.state, receipt::State::Failed);
    assert_eq!(
        receipt
            .transitions
            .last()
            .and_then(|transition| transition.incident.as_ref())
            .is_some(),
        incident
    );
}
