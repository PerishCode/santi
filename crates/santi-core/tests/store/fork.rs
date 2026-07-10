use super::support::*;

#[test]
fn fork_copies_prefix() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let parent = store.create_strand().expect("create parent");
    let first = store
        .append_message(
            &parent.id,
            ActorType::System,
            store.system_actor_id(),
            MessageContent::text("first"),
            MessageState::Fixed,
            MessageIntake::Record,
        )
        .expect("append first")
        .strand_message;
    let second = store
        .append_message(
            &parent.id,
            ActorType::System,
            store.system_actor_id(),
            MessageContent::text("second"),
            MessageState::Fixed,
            MessageIntake::Record,
        )
        .expect("append second")
        .strand_message;
    let third = store
        .append_message(
            &parent.id,
            ActorType::System,
            store.system_actor_id(),
            MessageContent::text("third"),
            MessageState::Fixed,
            MessageIntake::Record,
        )
        .expect("append third")
        .strand_message;

    let child = store.fork_strand(&parent.id, 2).expect("fork");
    assert_eq!(child.parent_strand_id.as_deref(), Some(parent.id.as_str()));
    assert_eq!(child.fork_point, Some(2));
    assert_eq!(child.next_seq, 3);
    assert!(child.external_label.is_none());
    assert!(child.provider_state.is_none());

    let child_messages = store.strand_messages(&child.id).expect("child messages");
    assert_eq!(child_messages.len(), 2);
    assert_eq!(child_messages[0].relation.strand_seq, 1);
    assert_eq!(child_messages[1].relation.strand_seq, 2);
    assert_eq!(child_messages[0].message.id, first.message.id);
    assert_eq!(child_messages[1].message.id, second.message.id);
    assert_eq!(child_messages[0].content_text, "first");
    assert_eq!(child_messages[1].content_text, "second");
    assert!(
        !child_messages
            .iter()
            .any(|m| m.message.id == third.message.id)
    );
}

#[test]
fn fork_copies_inner_compacts() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let parent = store.create_strand().expect("create parent");
    let mut messages = Vec::new();
    for text in ["one", "two", "three", "four"] {
        messages.push(
            store
                .append_message(
                    &parent.id,
                    ActorType::System,
                    store.system_actor_id(),
                    MessageContent::text(text),
                    MessageState::Fixed,
                    MessageIntake::Record,
                )
                .expect("append")
                .strand_message,
        );
    }
    let inside = store
        .create_compact(
            &parent.id,
            &messages[0].message.id,
            &messages[1].message.id,
            "inside",
        )
        .expect("inside compact");
    let crossing = store
        .create_compact(
            &parent.id,
            &messages[2].message.id,
            &messages[3].message.id,
            "crossing",
        )
        .expect("crossing compact");

    let child = store.fork_strand(&parent.id, 3).expect("fork");
    let snapshot = store
        .runtime_snapshot(&child.id)
        .expect("snapshot")
        .expect("child snapshot");
    assert_eq!(snapshot.compacts.len(), 1);
    assert_ne!(snapshot.compacts[0].id, inside.compact_id);
    assert_eq!(snapshot.compacts[0].strand_id, child.id);
    assert_eq!(snapshot.compacts[0].summary, "inside");
    assert_eq!(
        snapshot.compacts[0].start_message_id,
        inside.start_message_id
    );
    assert_eq!(snapshot.compacts[0].end_message_id, inside.end_message_id);
    assert_ne!(snapshot.compacts[0].id, crossing.compact_id);

    let input = store.assembly_input(&child.id).expect("child input");
    assert_eq!(input.len(), 2);
    let ProviderItem::Message { role, content } = &input[0] else {
        panic!("expected compact projection message");
    };
    assert_eq!(role, "system");
    assert!(content.contains("[compact projection]"));
    assert!(content.contains("\"operation\": \"compact_projection\""));
    assert!(content.contains("\"declared_source\": \"not_declared\""));
    assert!(content.contains("\"compact_id\""));
    assert!(content.contains(&snapshot.compacts[0].id));
    assert!(content.contains(&inside.start_message_id));
    assert!(content.contains(&inside.end_message_id));
    assert!(content.contains("<compact_summary>\ninside\n</compact_summary>"));
    assert_text(&input[1], "user", "three");
}

#[test]
fn fork_reuses_tools() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let parent = store.create_strand().expect("create parent");
    let user = store
        .append_message(
            &parent.id,
            ActorType::System,
            store.system_actor_id(),
            MessageContent::text("run"),
            MessageState::Fixed,
            MessageIntake::Request,
        )
        .expect("append user")
        .strand_message;
    let turn = store
        .start_turn(&parent.id, &user.message.id)
        .expect("start turn")
        .turn;
    store
        .append_tool_call(
            &turn.id,
            "call_fork",
            "shell",
            &serde_json::json!({ "command": "echo fork" }),
            &ToolCallProvenance {
                provider_family: "openai".to_string(),
                item: Some(serde_json::json!({ "type": "function_call", "id": "fc_fork" })),
                item_id: Some("fc_fork".to_string()),
                response_id: Some("resp_fork".to_string()),
            },
        )
        .expect("append tool call");
    store
        .append_tool_result(
            "call_fork",
            Some(serde_json::json!({ "stdout": "fork\n" })),
            None,
        )
        .expect("append tool result");

    let child = store.fork_strand(&parent.id, 3).expect("fork");
    let snapshot = store
        .runtime_snapshot(&child.id)
        .expect("snapshot")
        .expect("child snapshot");
    assert_eq!(snapshot.tool_calls.len(), 1);
    assert_eq!(snapshot.tool_calls[0].id, "call_fork");
    assert_eq!(snapshot.tool_results.len(), 1);
    assert_eq!(snapshot.tool_results[0].tool_call_id, "call_fork");

    let child_input = store.assembly_input(&child.id).expect("child input");
    assert_eq!(child_input.len(), 3);
    match &child_input[1] {
        ProviderItem::FunctionCall {
            call_id,
            item_id,
            item,
            ..
        } => {
            assert_eq!(call_id, "call_fork");
            assert_eq!(item_id.as_deref(), Some("fc_fork"));
            assert_eq!(item.as_ref().expect("raw item")["id"], "fc_fork");
        }
        other => panic!("expected function call, got {other:?}"),
    }
    match &child_input[2] {
        ProviderItem::FunctionCallOutput { call_id, output } => {
            assert_eq!(call_id, "call_fork");
            assert!(output.contains("fork"));
        }
        other => panic!("expected function call output, got {other:?}"),
    }
}

#[test]
fn fork_drops_external_state() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let parent = store
        .find_labeled_strand(store.default_soul_id(), "github:issue:fork")
        .expect("label strand");
    assert_eq!(parent.external_label.as_deref(), Some("github:issue:fork"));
    let turn = store
        .start_turn(&parent.id, "manual")
        .expect("start turn")
        .turn;
    store
        .complete_turn(
            &turn.id,
            None,
            "fake-provider",
            Some("resp_parent".to_string()),
        )
        .expect("complete turn");
    let parent = store
        .strand(&parent.id)
        .expect("load parent")
        .expect("parent");
    assert!(parent.provider_state.is_some());

    let child = store.fork_strand(&parent.id, 0).expect("fork empty prefix");

    assert_eq!(child.parent_strand_id.as_deref(), Some(parent.id.as_str()));
    assert_eq!(child.fork_point, Some(0));
    assert!(child.external_label.is_none());
    assert!(child.provider_state.is_none());
}
