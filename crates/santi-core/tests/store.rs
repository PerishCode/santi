use rusqlite::Connection;
use santi_core::{
    ActorType, IngestOutcome, MessageContent, MessageIntake, MessageKind, MessageState,
    ProviderItem, SantiStore, ThinkingCompletionReason, ToolCallProvenance,
};

fn assert_text(item: &ProviderItem, role: &str, content: &str) {
    match item {
        ProviderItem::Message {
            role: actual_role,
            content: actual_content,
        } => {
            assert_eq!(actual_role, role);
            assert_eq!(actual_content, content);
        }
        other => panic!("expected text item, got {other:?}"),
    }
}

#[test]
fn schema_matches_runtime() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    let store = SantiStore::open(&db).expect("open store");
    drop(store);

    let conn = Connection::open(db).expect("open sqlite");
    for table in [
        "souls",
        "messages",
        "message_events",
        "strand_effects",
        "strands",
        "strand_inbox",
        "turns",
        "tool_calls",
        "tool_results",
        "thinking_spans",
        "compacts",
        "r_strand_entries",
    ] {
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .expect("table lookup");
        assert_eq!(exists, 1, "missing table {table}");
    }
    // The discarded tables keep their historical (pre-rename) names — these
    // are the OLD session-era tables that must NOT exist in the clean schema.
    for table in [
        "accounts",
        "soul_profiles",
        "soul_sessions",
        "sessions",
        "session_profiles",
        "r_session_messages",
        "session_effects",
    ] {
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .expect("table lookup");
        assert_eq!(exists, 0, "discarded table {table} still present");
    }
}

#[test]
fn appends_relations_in_order() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let strand = store.create_strand().expect("create strand");
    let user = store
        .append_message(
            &strand.id,
            ActorType::System,
            store.system_actor_id(),
            MessageContent::text("hello ordering"),
            MessageState::Fixed,
            MessageIntake::Request,
        )
        .expect("append user")
        .strand_message;

    assert_eq!(user.relation.strand_seq, 1);
    let input = store.assembly_input(&strand.id).expect("assembly input");
    assert_eq!(input.len(), 1);
    assert_text(&input[0], "user", "hello ordering");
}

#[test]
fn maps_santi_system_input() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let strand = store.create_strand().expect("create strand");
    let message = store
        .append_santi_system_message(
            &strand.id,
            MessageContent::text("<system_message>\nkind: note\n</system_message>"),
            MessageIntake::Request,
        )
        .expect("append santi system")
        .strand_message;

    assert_eq!(message.message.actor_type, ActorType::System);
    assert_eq!(message.message.message_kind, MessageKind::SantiSystem);
    let input = store.assembly_input(&strand.id).expect("assembly input");
    assert_eq!(input.len(), 1);
    assert_text(
        &input[0],
        "system",
        "<system_message>\nkind: note\n</system_message>",
    );
}

#[test]
fn thinking_spans_become_reasoning_items() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let strand = store.create_strand().expect("create strand");
    let user = store
        .append_message(
            &strand.id,
            ActorType::System,
            store.system_actor_id(),
            MessageContent::text("hello thinking"),
            MessageState::Fixed,
            MessageIntake::Request,
        )
        .expect("append user")
        .strand_message;
    let turn = store
        .start_turn(&strand.id, &user.message.id)
        .expect("start turn")
        .turn;
    let thinking = store
        .append_thinking_span(&turn.id, Some("resp_test".to_string()))
        .expect("append thinking");
    let thinking = store
        .update_thinking_span_summary(&thinking.id, "Looked at the prompt.".to_string())
        .expect("update thinking summary")
        .expect("thinking exists");
    let thinking = store
        .complete_thinking_span(&thinking.id, ThinkingCompletionReason::FirstTextDelta)
        .expect("complete thinking")
        .expect("thinking exists");

    let snapshot = store
        .runtime_snapshot(&strand.id)
        .expect("runtime snapshot")
        .expect("strand exists");
    assert_eq!(snapshot.thinking_spans.len(), 1);
    assert_eq!(snapshot.thinking_spans[0].id, thinking.id);
    assert_eq!(
        snapshot.thinking_spans[0].provider_response_id.as_deref(),
        Some("resp_test")
    );
    assert_eq!(
        snapshot.thinking_spans[0].summary.as_deref(),
        Some("Looked at the prompt.")
    );
    assert_eq!(
        snapshot.thinking_spans[0].completion_reason,
        Some(ThinkingCompletionReason::FirstTextDelta)
    );

    // Reasoning is now a first-class timeline item (adapters drop it per DC5,
    // but the projection includes it when there is real summary text).
    let input = store.assembly_input(&strand.id).expect("assembly input");
    assert_eq!(input.len(), 2);
    assert_text(&input[0], "user", "hello thinking");
    match &input[1] {
        ProviderItem::Reasoning { id, content } => {
            assert_eq!(id.as_deref(), Some("resp_test"));
            assert_eq!(content, "Looked at the prompt.");
        }
        other => panic!("expected reasoning item, got {other:?}"),
    }
}

#[test]
fn projects_timeline_to_interleaved_items() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let strand = store.create_strand().expect("create strand");

    // A turn with a shell tool roundtrip and per-round assistant text: the
    // replay timeline interleaves user, function call, result, assistant text.
    let user = store
        .append_message(
            &strand.id,
            ActorType::System,
            store.system_actor_id(),
            MessageContent::text("run a command"),
            MessageState::Fixed,
            MessageIntake::Request,
        )
        .expect("append user")
        .strand_message;
    let turn = store
        .start_turn(&strand.id, &user.message.id)
        .expect("start turn")
        .turn;
    store
        .append_tool_call(
            &turn.id,
            "call_1",
            "shell",
            &serde_json::json!({ "command": "echo hi" }),
            &ToolCallProvenance {
                item: Some(serde_json::json!({ "type": "function_call", "id": "fc_1" })),
                item_id: Some("fc_1".to_string()),
                response_id: Some("resp_1".to_string()),
            },
        )
        .expect("append tool call");
    store
        .append_tool_result(
            "call_1",
            Some(serde_json::json!({ "stdout": "hi\n" })),
            None,
        )
        .expect("append tool result");
    // The final assistant text is a soul-only timeline item (DC4b); the lumped
    // strand-visible reply is stored separately by the service.
    store
        .append_soul_assistant_text(&strand.id, "done")
        .expect("append soul assistant text");

    let input = store.assembly_input(&strand.id).expect("assembly input");
    assert_eq!(input.len(), 4);
    assert_text(&input[0], "user", "run a command");
    match &input[1] {
        ProviderItem::FunctionCall {
            call_id,
            name,
            arguments_raw,
            item,
            item_id,
        } => {
            assert_eq!(call_id, "call_1");
            assert_eq!(name, "shell");
            assert!(arguments_raw.contains("echo hi"));
            // The raw provider item + id round-trip for faithful Responses replay.
            assert_eq!(item_id.as_deref(), Some("fc_1"));
            assert_eq!(item.as_ref().expect("raw item")["id"], "fc_1");
        }
        other => panic!("expected function call, got {other:?}"),
    }
    match &input[2] {
        ProviderItem::FunctionCallOutput { call_id, output } => {
            assert_eq!(call_id, "call_1");
            assert!(output.contains("\"ok\":true"));
            assert!(output.contains("hi"));
        }
        other => panic!("expected function call output, got {other:?}"),
    }
    assert_text(&input[3], "assistant", "done");
}

/// A REQUEST enters through ingest (the inbox) — that's what makes a strand
/// "behind" now. A RECORD (the soul's own output, a failure notice) bypasses
/// the inbox entirely and is written straight to the timeline, same as
/// `append_soul_assistant_text`/`fail_background_turn` do in the real service.
fn append_timeline_message(
    store: &SantiStore,
    strand_id: &str,
    actor_type: ActorType,
    text: &str,
    intake: MessageIntake,
) {
    match intake {
        MessageIntake::Request => {
            store
                .enqueue_inbox(strand_id, MessageKind::Text, MessageContent::text(text))
                .expect("enqueue inbox");
        }
        MessageIntake::Record => {
            let actor_id = match actor_type {
                ActorType::Soul => store.default_soul_id(),
                ActorType::System => store.system_actor_id(),
            };
            store
                .append_message(
                    strand_id,
                    actor_type,
                    actor_id,
                    MessageContent::text(text),
                    MessageState::Fixed,
                    intake,
                )
                .expect("append message");
        }
    }
}

#[test]
fn drive_starts_coalesces_and_re_drives() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let strand = store.create_strand().expect("create strand");

    // No requests → not behind → no turn.
    assert!(
        store
            .try_start_turn(&strand.id, "strand_send", None)
            .expect("try")
            .is_none()
    );

    // A REQUEST makes the thread behind → starts a turn.
    append_timeline_message(
        &store,
        &strand.id,
        ActorType::System,
        "hi",
        MessageIntake::Request,
    );
    let started = store
        .try_start_turn(&strand.id, "strand_send", None)
        .expect("try")
        .expect("turn started");
    assert_eq!(started.turn.status, santi_core::TurnStatus::Running);
    assert_eq!(started.drained_messages.len(), 1);
    let turn = started.turn;

    // A second request while the turn runs coalesces — no concurrent turn.
    append_timeline_message(
        &store,
        &strand.id,
        ActorType::System,
        "and again",
        MessageIntake::Request,
    );
    assert!(
        store
            .try_start_turn(&strand.id, "strand_send", None)
            .expect("try")
            .is_none(),
        "a running turn must block a second concurrent turn"
    );

    // After the turn completes, the request that arrived during it is past the
    // turn's start → behind again → drive the next turn.
    store
        .complete_turn(&turn.id, None, "fake", None)
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
fn drain_commits_all_pending_inbox_entries_to_one_turn() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let strand = store.create_strand().expect("create strand");

    // Multiple adaptors can enqueue concurrently before the driver ever runs;
    // the NEXT drive drains everything present into ONE turn, in arrival order.
    for text in ["first", "second", "third"] {
        store
            .enqueue_inbox(&strand.id, MessageKind::Text, MessageContent::text(text))
            .expect("enqueue");
    }
    let started = store
        .try_start_turn(&strand.id, "strand_send", None)
        .expect("try")
        .expect("turn started");
    assert_eq!(started.drained_messages.len(), 3);
    assert_eq!(started.drained_messages[0].content_text, "first");
    assert_eq!(started.drained_messages[1].content_text, "second");
    assert_eq!(started.drained_messages[2].content_text, "third");
    for (index, message) in started.drained_messages.iter().enumerate() {
        assert_eq!(message.relation.strand_seq, (index + 1) as i64);
    }

    // The inbox is now empty — nothing left to drain, no new turn.
    assert!(
        store
            .try_start_turn(&strand.id, "strand_send", None)
            .expect("try")
            .is_none()
    );
}

#[test]
fn inbox_gate_rejects_past_threshold() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let strand = store.create_strand().expect("create strand");

    // Never drained (no try_start_turn call), so every enqueue adds to the
    // undrained count — eventually the gate must start rejecting rather than
    // growing without bound.
    let mut rejected = false;
    for _ in 0..600 {
        match store
            .enqueue_inbox(&strand.id, MessageKind::Text, MessageContent::text("x"))
            .expect("enqueue")
        {
            IngestOutcome::Accepted { .. } => {}
            IngestOutcome::Rejected { reason } => {
                assert!(reason.contains("inbox is full"), "got: {reason}");
                rejected = true;
                break;
            }
        }
    }
    assert!(rejected, "gate never rejected after 600 enqueues");
}

#[test]
fn record_messages_do_not_drive() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let strand = store.create_strand().expect("create strand");

    // A RECORD (the soul's own output / a failure notice) is not a request and
    // must not wake the soul.
    append_timeline_message(
        &store,
        &strand.id,
        ActorType::Soul,
        "a note to self",
        MessageIntake::Record,
    );
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
fn boot_recovery_reconciles_and_does_not_retry() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let strand = store.create_strand().expect("create strand");
    append_timeline_message(
        &store,
        &strand.id,
        ActorType::System,
        "do a thing",
        MessageIntake::Request,
    );
    // A turn starts and then the process "crashes" (turn left running).
    store
        .try_start_turn(&strand.id, "strand_send", None)
        .expect("try")
        .expect("turn started");
    // Before recovery it is the only pending-driver; reconcile interrupts it.
    assert_eq!(store.reconcile_orphaned_turns().expect("reconcile"), 1);
    // The interrupted turn counts as "attempted" → the request is NOT retried.
    assert!(
        store
            .try_start_turn(&strand.id, "strand_send", None)
            .expect("try")
            .is_none(),
        "an interrupted turn must not auto-retry its request"
    );
    // But a genuinely new request drives a fresh turn (liveness).
    append_timeline_message(
        &store,
        &strand.id,
        ActorType::System,
        "a new thing",
        MessageIntake::Request,
    );
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

#[test]
fn create_soul_and_label_anchoring() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");

    // Souls are API-managed individuals, id-only; a created soul is distinct
    // from default and shows up in the roster.
    let soul = store.create_soul().expect("create soul");
    assert_ne!(soul.id, store.default_soul_id());
    assert!(store.list_souls().expect("list").len() >= 2);
    assert!(store.soul(&soul.id).expect("soul").is_some());

    // External label anchors a strand (scoped to its soul): same label → same
    // strand; new → new; a different soul on the same label gets its own strand.
    let s1 = store
        .find_or_create_strand_by_label(&soul.id, "github:issue:49")
        .expect("label strand");
    let s1_again = store
        .find_or_create_strand_by_label(&soul.id, "github:issue:49")
        .expect("label strand again");
    assert_eq!(s1.id, s1_again.id);
    let s2 = store
        .find_or_create_strand_by_label(&soul.id, "github:issue:50")
        .expect("other label");
    assert_ne!(s1.id, s2.id);
    assert_eq!(s1.soul_id, soul.id);
    assert_eq!(store.soul_id_for_strand(&s1.id).expect("soul id"), soul.id);

    let default_strand = store
        .find_or_create_strand_by_label(store.default_soul_id(), "github:issue:49")
        .expect("same label, default soul");
    assert_ne!(default_strand.id, s1.id);

    // An unknown soul cannot anchor a new label.
    assert!(
        store
            .find_or_create_strand_by_label("soul_does_not_exist", "github:issue:99")
            .is_err()
    );
}

#[test]
fn read_schema_version_none_when_db_absent() {
    let temp = tempfile::tempdir().expect("temp dir");
    let missing = temp.path().join("nope.sqlite");
    assert_eq!(
        santi_core::read_schema_version(&missing).expect("read"),
        None
    );
}

#[test]
fn read_schema_version_is_readonly_and_matches_after_open() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");

    // A DB stamped at a stale version: the probe reports it AS-IS and, crucially,
    // does NOT migrate/wipe it (unlike SantiStore::open).
    {
        let conn = Connection::open(&db).expect("open sqlite");
        conn.pragma_update(None, "user_version", 5u32)
            .expect("stamp version");
    }
    assert_eq!(
        santi_core::read_schema_version(&db).expect("read"),
        Some(5),
        "probe must report the stored version, not migrate it"
    );
    assert_eq!(
        santi_core::read_schema_version(&db).expect("read again"),
        Some(5),
        "a second probe still sees the stale version — the first was read-only"
    );

    // Opening the store DOES migrate to the runtime's version.
    let store = SantiStore::open(&db).expect("open store");
    drop(store);
    assert_eq!(
        santi_core::read_schema_version(&db).expect("read post-open"),
        Some(santi_core::SCHEMA_VERSION)
    );
}

#[test]
fn soul_memory_file_composes_under_runtime_root() {
    let path = santi_core::soul_memory_file("/srv/santi/runtime", "soul_default");
    assert!(path.ends_with("souls/soul_default/memory/MEMORY.md"));
    assert!(path.starts_with("/srv/santi/runtime"));
}
