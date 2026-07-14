use super::support::*;

struct StartedEffect {
    turn_id: String,
    effect_id: String,
    inbox_id: String,
}

fn start_effect(store: &SantiStore) -> StartedEffect {
    let strand = store.create_strand().expect("create strand");
    let inbox_id = match store
        .enqueue_inbox(
            &strand.id,
            MessageKind::Text,
            MessageContent::text("run an external effect"),
        )
        .expect("enqueue")
    {
        IngestOutcome::Accepted { receipt } => receipt.inbox_id,
        IngestOutcome::Rejected { .. } => panic!("unexpected rejection"),
    };
    let turn = store
        .try_start_turn(&strand.id, "strand_send", None)
        .expect("start turn")
        .expect("started turn")
        .turn;
    let tool_call_id = "call_effect".to_string();
    let (_, effect) = store
        .append_effect_call(
            Invocation {
                turn: &turn.id,
                call: &tool_call_id,
                name: "shell",
                arguments: &json!({"command": "printf external"}),
                provenance: &ToolCallProvenance::default(),
            },
            Some("shell"),
        )
        .expect("append effect intent");
    StartedEffect {
        turn_id: turn.id,
        effect_id: effect.expect("effect").id,
        inbox_id,
    }
}

#[test]
fn prepared_failure() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let started = start_effect(&store);

    store
        .fail_turn(&started.turn_id, "failure before dispatch")
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
        .receipt_status(&started.inbox_id)
        .expect("query receipt")
        .expect("receipt");
    assert_eq!(receipt.state, ReceiptState::TurnFailed);
    assert_eq!(receipt.effects.len(), 1);
    assert_eq!(receipt.effects[0].state, EffectState::NotDispatched);
}

#[test]
fn restart_ambiguity() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = SantiStore::open(temp.path().join("santi.sqlite")).expect("open store");
    let started = start_effect(&store);
    store
        .begin_effect_dispatch(&started.effect_id)
        .expect("open dispatch window");
    let (_, prepared) = store
        .append_effect_call(
            Invocation {
                turn: &started.turn_id,
                call: "call_still_prepared",
                name: "shell",
                arguments: &json!({"command": "printf prepared"}),
                provenance: &ToolCallProvenance::default(),
            },
            Some("shell"),
        )
        .expect("append second effect");
    let prepared_id = prepared.expect("prepared effect").id;

    assert_eq!(store.reconcile_orphaned_turns().expect("reconcile"), 1);
    let status = store
        .effect_status(&started.effect_id)
        .expect("query effect")
        .expect("effect");
    assert_eq!(status.effect.state, EffectState::Unknown);
    assert_eq!(
        status
            .transitions
            .iter()
            .map(|transition| transition.reason.clone())
            .collect::<Vec<_>>(),
        vec![
            EffectTransitionReason::IntentPersisted,
            EffectTransitionReason::DispatchWindowOpened,
            EffectTransitionReason::RestartDuringDispatch,
        ]
    );
    let prepared = store
        .effect_status(&prepared_id)
        .expect("query prepared effect")
        .expect("prepared effect");
    assert_eq!(prepared.effect.state, EffectState::NotDispatched);
    assert_eq!(
        prepared
            .transitions
            .last()
            .expect("restart transition")
            .reason,
        EffectTransitionReason::RestartBeforeDispatch
    );
    assert!(
        store
            .tool_results_for_turn(&started.turn_id)
            .expect("tool results before resolution")
            .is_empty()
    );

    let resolved = store
        .resolve_effect(
            &started.effect_id,
            EffectResolutionOutcome::NotApplied,
            "operator checked the target system",
        )
        .expect("resolve")
        .expect("resolved effect");
    assert_eq!(resolved.effect.state, EffectState::ResolvedNotApplied);
    assert_eq!(
        resolved.transitions.last().expect("resolution").reason,
        EffectTransitionReason::OperatorResolvedNotApplied
    );
    assert_eq!(
        resolved
            .transitions
            .last()
            .expect("resolution")
            .evidence
            .as_deref(),
        Some("operator checked the target system")
    );
    assert!(
        store
            .tool_results_for_turn(&started.turn_id)
            .expect("tool results after resolution")
            .is_empty(),
        "resolution records evidence; it must not dispatch or fabricate a tool result"
    );
    assert!(
        store
            .resolve_effect(
                &started.effect_id,
                EffectResolutionOutcome::Applied,
                "second guess",
            )
            .is_err(),
        "a settled operator resolution is immutable"
    );
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

#[test]
fn v25_effect_import() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    drop(SantiStore::open(&db).expect("create current store"));

    let conn = Connection::open(&db).expect("open sqlite");
    conn.execute_batch(
        r#"
        DROP TABLE effect_transitions;
        DROP TABLE strand_effects;
        CREATE TABLE strand_effects (
            id TEXT PRIMARY KEY,
            strand_id TEXT NOT NULL,
            effect_type TEXT NOT NULL,
            idempotency_key TEXT NOT NULL,
            status TEXT NOT NULL,
            source_hook_id TEXT NOT NULL,
            source_turn_id TEXT NOT NULL,
            result_ref TEXT,
            error_text TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE (strand_id, effect_type, idempotency_key)
        );
        INSERT INTO strand_effects (
            id, strand_id, effect_type, idempotency_key, status,
            source_hook_id, source_turn_id, result_ref, error_text,
            created_at, updated_at
        ) VALUES (
            'effect_legacy', 'ss_legacy', 'shell', 'legacy-key', 'completed',
            'hook_legacy', 'turn_legacy', 'legacy-result', NULL,
            '2026-07-01T00:00:00Z', '2026-07-01T00:00:01Z'
        );
        PRAGMA user_version = 25;
        "#,
    )
    .expect("seed v25 effect row");
    drop(conn);

    let store = SantiStore::open(&db).expect("migrate v25 to v26");
    let status = store
        .effect_status("effect_legacy")
        .expect("query migrated effect")
        .expect("migrated effect");
    assert_eq!(status.effect.state, EffectState::Unknown);
    assert_eq!(status.effect.turn_id, "turn_legacy");
    assert_eq!(status.effect.tool_call_id, None);
    assert_eq!(status.effect.result_ref.as_deref(), Some("legacy-result"));
    assert_eq!(status.transitions.len(), 1);
    assert_eq!(
        status.transitions[0].reason,
        EffectTransitionReason::LegacyImport
    );
    let evidence: serde_json::Value = serde_json::from_str(
        status.transitions[0]
            .evidence
            .as_deref()
            .expect("legacy evidence"),
    )
    .expect("legacy evidence json");
    assert_eq!(evidence["legacy_v25"]["idempotency_key"], "legacy-key");
    assert_eq!(evidence["legacy_v25"]["status"], "completed");
    assert_eq!(evidence["legacy_v25"]["source_hook_id"], "hook_legacy");
    assert_eq!(status.receipt_ids, Vec::<String>::new());
    assert_eq!(
        santi_core::read_schema_version(&db).expect("read schema"),
        Some(santi_core::SCHEMA_VERSION)
    );
}
