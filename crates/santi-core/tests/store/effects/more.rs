use super::*;

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
                turn: &started.turn,
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
            .tool_results_for_turn(&started.turn)
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
            .tool_results_for_turn(&started.turn)
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
