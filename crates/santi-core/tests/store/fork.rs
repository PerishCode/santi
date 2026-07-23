use super::support::*;
use santi_core::{message, tool};

#[test]
fn fork_copies_prefix() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = Store::open(temp.path().join("santi.sqlite")).expect("open store");
    let parent = store.weave().expect("create parent");
    let first = store
        .pen(Draft {
            strand: &parent.id,
            actor: message::Role::System,
            id: store.system(),
            content: message::Content::text("first"),
            state: message::State::Fixed,
            intake: message::Intake::Record,
        })
        .expect("append first")
        .message;
    let second = store
        .pen(Draft {
            strand: &parent.id,
            actor: message::Role::System,
            id: store.system(),
            content: message::Content::text("second"),
            state: message::State::Fixed,
            intake: message::Intake::Record,
        })
        .expect("append second")
        .message;
    let third = store
        .pen(Draft {
            strand: &parent.id,
            actor: message::Role::System,
            id: store.system(),
            content: message::Content::text("third"),
            state: message::State::Fixed,
            intake: message::Intake::Record,
        })
        .expect("append third")
        .message;

    let child = store.fork(&parent.id, 2).expect("fork");
    assert_eq!(child.parent.as_deref(), Some(parent.id.as_str()));
    assert_eq!(child.fork, Some(2));
    assert_eq!(child.next, 3);
    assert!(child.label.is_none());
    assert!(child.state.is_none());

    let child_messages = store.messages(&child.id).expect("child messages");
    assert_eq!(child_messages.len(), 2);
    assert_eq!(child_messages[0].relation.seq, 1);
    assert_eq!(child_messages[1].relation.seq, 2);
    assert_eq!(child_messages[0].message.id, first.message.id);
    assert_eq!(child_messages[1].message.id, second.message.id);
    assert_eq!(child_messages[0].text, "first");
    assert_eq!(child_messages[1].text, "second");
    assert!(
        !child_messages
            .iter()
            .any(|m| m.message.id == third.message.id)
    );
}

#[test]
fn fork_copies_inner_compacts() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = Store::open(temp.path().join("santi.sqlite")).expect("open store");
    let parent = store.weave().expect("create parent");
    let mut messages = Vec::new();
    for text in ["one", "two", "three", "four"] {
        messages.push(
            store
                .pen(Draft {
                    strand: &parent.id,
                    actor: message::Role::System,
                    id: store.system(),
                    content: message::Content::text(text),
                    state: message::State::Fixed,
                    intake: message::Intake::Record,
                })
                .expect("append")
                .message,
        );
    }
    let inside = store
        .compact(
            &parent.id,
            &messages[0].message.id,
            &messages[1].message.id,
            "inside",
        )
        .expect("inside compact");
    let crossing = store
        .compact(
            &parent.id,
            &messages[2].message.id,
            &messages[3].message.id,
            "crossing",
        )
        .expect("crossing compact");

    let child = store.fork(&parent.id, 3).expect("fork");
    let snapshot = store
        .snapshot(&child.id)
        .expect("snapshot")
        .expect("child snapshot");
    assert_eq!(snapshot.compacts.len(), 1);
    assert_ne!(snapshot.compacts[0].id, inside.compact);
    assert_eq!(snapshot.compacts[0].strand, child.id);
    assert_eq!(snapshot.compacts[0].summary, "inside");
    assert_eq!(snapshot.compacts[0].first, inside.first);
    assert_eq!(snapshot.compacts[0].last, inside.last);
    assert_ne!(snapshot.compacts[0].id, crossing.compact);

    let input = store.assembly(&child.id).expect("child input");
    assert_eq!(input.len(), 2);
    let Item::Message { role, content } = &input[0] else {
        panic!("expected compact projection message");
    };
    assert_eq!(role, "system");
    assert!(content.contains("[compact projection]"));
    assert!(content.contains("\"operation\": \"compact_projection\""));
    assert!(content.contains("\"declared_source\": \"not_declared\""));
    assert!(content.contains("\"compact\""));
    assert!(content.contains(&snapshot.compacts[0].id));
    assert!(content.contains(&inside.first));
    assert!(content.contains(&inside.last));
    assert!(content.contains("<compact_summary>\ninside\n</compact_summary>"));
    assert_text(&input[1], "user", "three");
}

#[test]
fn fork_reuses_tools() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = Store::open(temp.path().join("santi.sqlite")).expect("open store");
    let parent = store.weave().expect("create parent");
    let user = store
        .pen(Draft {
            strand: &parent.id,
            actor: message::Role::System,
            id: store.system(),
            content: message::Content::text("run"),
            state: message::State::Fixed,
            intake: message::Intake::Request,
        })
        .expect("append user")
        .message;
    let turn = store
        .start(&parent.id, &user.message.id)
        .expect("start turn")
        .turn;
    store
        .call(Invocation {
            turn: &turn.id,
            call: "call_fork",
            name: "shell",
            arguments: &serde_json::json!({ "command": "echo fork" }),
            provenance: &tool::Provenance {
                family: "openai".to_string(),
                item: Some(serde_json::json!({ "type": "function_call", "id": "fc_fork" })),
                mark: Some("fc_fork".to_string()),
                response: Some("resp_fork".to_string()),
            },
        })
        .expect("append tool call");
    store
        .reply(
            "call_fork",
            Some(serde_json::json!({ "stdout": "fork\n" })),
            None,
        )
        .expect("append tool result");

    let child = store.fork(&parent.id, 3).expect("fork");
    let snapshot = store
        .snapshot(&child.id)
        .expect("snapshot")
        .expect("child snapshot");
    assert_eq!(snapshot.calls.len(), 1);
    assert_eq!(snapshot.calls[0].id, "call_fork");
    assert_eq!(snapshot.results.len(), 1);
    assert_eq!(snapshot.results[0].call, "call_fork");

    let child_input = store.assembly(&child.id).expect("child input");
    assert_eq!(child_input.len(), 3);
    match &child_input[1] {
        Item::Call {
            call, mark, item, ..
        } => {
            assert_eq!(call, "call_fork");
            assert_eq!(mark.as_deref(), Some("fc_fork"));
            assert_eq!(item.as_ref().expect("raw item")["id"], "fc_fork");
        }
        other => panic!("expected function call, got {other:?}"),
    }
    match &child_input[2] {
        Item::Output { call, output } => {
            assert_eq!(call, "call_fork");
            assert!(output.contains("fork"));
        }
        other => panic!("expected function call output, got {other:?}"),
    }
}

#[test]
fn fork_drops_external_state() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = Store::open(temp.path().join("santi.sqlite")).expect("open store");
    let parent = store
        .labeled(store.genesis(), "github:issue:fork")
        .expect("label strand");
    assert_eq!(parent.label.as_deref(), Some("github:issue:fork"));
    let turn = store.start(&parent.id, "manual").expect("start turn").turn;
    store
        .complete(Completion {
            turn: &turn.id,
            sequence: None,
            provider: "fake-provider",
            model: "fake-model",
            response: Some("resp_parent".to_string()),
        })
        .expect("complete turn");
    let parent = store
        .strand(&parent.id)
        .expect("load parent")
        .expect("parent");
    assert!(parent.state.is_some());

    let child = store.fork(&parent.id, 0).expect("fork empty prefix");

    assert_eq!(child.parent.as_deref(), Some(parent.id.as_str()));
    assert_eq!(child.fork, Some(0));
    assert!(child.label.is_none());
    assert!(child.state.is_none());
}
