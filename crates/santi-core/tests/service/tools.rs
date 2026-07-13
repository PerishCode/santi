use super::support::*;

#[tokio::test]
async fn dispatches_tools() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(FakeProvider {
        request_tool: true,
        ..FakeProvider::default()
    });
    let service = SantiService::open(
        SantiServiceConfig {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        provider.clone(),
    )
    .expect("open service");

    let strand = service.create_strand().expect("create strand").strand;
    let response = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "run tool".to_string(),
                }],
            },
        )
        .await
        .expect("send strand");

    assert_eq!(
        accepted_turn(&response).status,
        santi_core::TurnStatus::Running
    );
    let runtime = wait_for_completed_turn(&service, &strand.id, &accepted_turn(&response).id).await;
    assert!(
        runtime
            .messages
            .iter()
            .any(|message| message.content_text == "hi from runtime")
    );
    assert_eq!(runtime.tool_calls.len(), 1);
    assert_eq!(runtime.tool_calls[0].tool_name, "shell");
    assert_eq!(runtime.tool_results.len(), 1);
    assert!(runtime.tool_results[0].error_text.is_none());
    let output = runtime.tool_results[0]
        .output
        .as_ref()
        .expect("tool output");
    let stdout = output
        .get("stdout")
        .and_then(|value| value.as_str())
        .expect("shell stdout");
    let strand_memory_dir = Path::new("runtime")
        .join("strands")
        .join(&strand.id)
        .join("memory");
    assert!(stdout.contains(&strand_memory_dir.display().to_string()));
    // Self-involved env: the soul's shell inherits its own soul_id + strand_id,
    // so `santi …` from the shell auto-scopes to itself.
    assert!(
        stdout.contains("soul_default"),
        "SANTI_SOUL_ID in shell env: {stdout}"
    );
    assert!(
        stdout.contains(&strand.id),
        "SANTI_STRAND_ID in shell env: {stdout}"
    );
    let cwd = output
        .get("cwd")
        .and_then(|value| value.as_str())
        .expect("shell cwd");
    assert!(Path::new(cwd).ends_with(&strand_memory_dir));

    assert_eq!(runtime.effects.len(), 1);
    let effect = &runtime.effects[0];
    assert_eq!(effect.tool_call_id.as_deref(), Some("call_shell"));
    assert_eq!(effect.effect_type, "shell");
    assert_eq!(effect.state, EffectState::Confirmed);
    assert_eq!(
        effect.result_ref.as_deref(),
        Some(runtime.tool_results[0].id.as_str())
    );
    let effect_status = service
        .effect_status(&effect.id)
        .expect("query effect")
        .expect("shell effect");
    assert_eq!(
        effect_status
            .transitions
            .iter()
            .map(|transition| (&transition.state, &transition.reason))
            .collect::<Vec<_>>(),
        vec![
            (
                &EffectState::Prepared,
                &EffectTransitionReason::IntentPersisted
            ),
            (
                &EffectState::Dispatching,
                &EffectTransitionReason::DispatchWindowOpened,
            ),
            (
                &EffectState::Confirmed,
                &EffectTransitionReason::ResultPersisted
            ),
        ]
    );
    assert_eq!(
        effect_status.receipt_ids,
        vec![response.receipt.inbox_id.clone()]
    );
    let receipt = service
        .receipt_status(&response.receipt.inbox_id)
        .expect("query receipt")
        .expect("receipt");
    assert_eq!(receipt.effects.len(), 1);
    assert_eq!(receipt.effects[0].id, effect.id);
    assert_eq!(receipt.effects[0].state, EffectState::Confirmed);

    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].previous_response_id.is_none());
    // Round 2 re-derives input from the timeline: the prior tool call + result
    // are replayed as items (no function_call_outputs side-channel).
    assert!(
        requests[1]
            .input
            .iter()
            .any(|item| matches!(item, ProviderItem::FunctionCall { .. }))
    );
    assert!(
        requests[1]
            .input
            .iter()
            .any(|item| matches!(item, ProviderItem::FunctionCallOutput { .. }))
    );
}
