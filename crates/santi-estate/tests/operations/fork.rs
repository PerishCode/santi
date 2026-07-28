use santi_estate::{CompactDraft, ForkDraft, MessageDraft, Store, StrandDraft, TurnDraft};
use santi_model::{message, turn};

const FIRST: &str = "2026-07-28T00:00:00.000Z";
const LATER: &str = "2026-07-28T00:01:00.000Z";

#[tokio::test]
async fn forks_shared_occurrences() {
    let temp = tempfile::tempdir().expect("temp");
    let path = temp.path().join("estate.sqlite");
    let store = Store::open(&path).await.expect("open");
    store.seed("soul_test", FIRST).await.expect("seed");
    store
        .create_strand(StrandDraft {
            tag: "strand_parent",
            soul: "soul_test",
            label: Some("primary"),
            parent: None,
            fork: None,
            created: FIRST,
        })
        .await
        .expect("parent");
    for (tag, text) in [
        ("message_one", "one"),
        ("message_two", "two"),
        ("message_three", "three"),
        ("message_four", "four"),
    ] {
        store
            .place(MessageDraft {
                tag,
                strand: "strand_parent",
                actor: message::Role::System,
                actor_id: "santi",
                kind: message::Kind::Text,
                content: &message::Content::text(text),
                state: message::State::Fixed,
                request: false,
                created: FIRST,
            })
            .await
            .expect("message");
    }
    let inside = store
        .create_compact(CompactDraft {
            tag: "compact_inside",
            strand: "strand_parent",
            first: "message_one",
            last: "message_two",
            summary: "inside",
            metadata: Some(&serde_json::json!({"source": "parent"})),
            created: FIRST,
        })
        .await
        .expect("inside");
    store
        .create_compact(CompactDraft {
            tag: "compact_crossing",
            strand: "strand_parent",
            first: "message_three",
            last: "message_four",
            summary: "crossing",
            metadata: None,
            created: LATER,
        })
        .await
        .expect("crossing");

    let child = store
        .fork(ForkDraft {
            tag: "strand_child",
            parent: "strand_parent",
            at: 3,
            created: LATER,
        })
        .await
        .expect("fork");
    assert_eq!(child.parent.as_deref(), Some("strand_parent"));
    assert_eq!(child.fork, Some(3));
    assert_eq!(child.next, 4);
    assert_eq!(child.seen, 0);
    assert!(child.label.is_none());
    assert!(child.state.is_none());
    let messages = store.messages(&child.id).await.expect("messages");
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].message.id, "message_one");
    assert_eq!(messages[2].message.id, "message_three");
    let compacts = store.compacts(&child.id).await.expect("compacts");
    assert_eq!(compacts.len(), 1);
    assert_ne!(compacts[0].id, inside.compact);
    assert_eq!(compacts[0].summary, "inside");
    assert_eq!(
        compacts[0].metadata,
        Some(serde_json::json!({"source": "parent"}))
    );

    drop(store);
    let store = Store::open(&path).await.expect("open again");
    assert_eq!(
        store
            .snapshot("strand_child")
            .await
            .expect("snapshot")
            .expect("held")
            .messages
            .len(),
        3
    );
    assert!(store.discard_fork("strand_child").await.expect("discard"));
    assert!(store.strand("strand_child").await.expect("child").is_none());

    let child = store
        .fork(ForkDraft {
            tag: "strand_driven",
            parent: "strand_parent",
            at: 3,
            created: LATER,
        })
        .await
        .expect("fork driven");
    store
        .create_turn(TurnDraft {
            tag: "turn_child",
            strand: &child.id,
            trigger: turn::Trigger::System,
            source: None,
            from: 3,
            created: LATER,
        })
        .await
        .expect("turn");
    assert!(store.discard_fork(&child.id).await.is_err());
    assert!(store.discard_fork("strand_parent").await.is_err());
    assert!(!store.discard_fork("missing").await.expect("missing"));
}
