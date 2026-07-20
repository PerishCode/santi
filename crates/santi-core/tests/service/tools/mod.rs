use super::support::*;
use santi_core::service::{self, Service};

mod more;

#[derive(Clone)]
enum BudgetProviderStep {
    Calls { count: usize, output_bytes: usize },
    Fail(String),
    Complete,
}

#[derive(Clone)]
struct BudgetProvider {
    requests: Arc<Mutex<Vec<ProviderRequest>>>,
    steps: Arc<Vec<BudgetProviderStep>>,
}

#[async_trait]
impl ProviderClient for BudgetProvider {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            provider: Arc::from("budget-provider"),
            model: "budget-model".to_string(),
            context_budget: None,
        }
    }

    async fn stream_response(&self, request: ProviderRequest) -> Result<ProviderStream, String> {
        let index = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request);
            requests.len() - 1
        };
        let step = self
            .steps
            .get(index)
            .cloned()
            .unwrap_or(BudgetProviderStep::Complete);
        match step {
            BudgetProviderStep::Fail(error) => Err(error),
            BudgetProviderStep::Complete => {
                Ok(Box::pin(stream::iter(vec![Ok(ProviderEvent::Completed {
                    provider_response_id: Some(format!("resp_{index}")),
                })])))
            }
            BudgetProviderStep::Calls {
                count,
                output_bytes,
            } => Ok(Box::pin(stream::iter(tool_events(
                index,
                count,
                output_bytes,
            )))),
        }
    }
}

fn tool_events(
    index: usize,
    count: usize,
    output_bytes: usize,
) -> Vec<Result<ProviderEvent, String>> {
    let mut events = Vec::new();
    for call_index in 0..count {
        let call_id = format!("call_{index}_{call_index}");
        let response_id = format!("resp_{index}");
        let command = output_command(output_bytes);
        let arguments = json!({"command": command});
        let arguments_raw = arguments.to_string();
        events.push(Ok(ProviderEvent::FunctionCallRequested(
            ProviderFunctionCall {
                response_id: response_id.clone(),
                item_id: Some(format!("item_{call_id}")),
                item: json!({
                    "type": "function_call",
                    "id": format!("item_{call_id}"),
                    "call_id": call_id,
                    "name": "shell",
                    "arguments": arguments_raw,
                }),
                call_id,
                name: "shell".to_string(),
                arguments_raw,
                arguments,
            },
        )));
    }
    events.push(Ok(ProviderEvent::Completed {
        provider_response_id: Some(format!("resp_{index}")),
    }));
    events
}

fn output_command(bytes: usize) -> String {
    let output = "x".repeat(bytes);
    if cfg!(windows) {
        format!("[Console]::Out.Write('{output}')")
    } else {
        format!("printf '{output}'")
    }
}

fn execution_budget(
    max_provider_rounds: usize,
    max_tool_calls: usize,
    max_tool_output_bytes: usize,
    max_shell_output_bytes: usize,
) -> santi_core::Execution {
    santi_core::Execution {
        profile: "test_budget".to_string(),
        max_provider_rounds,
        max_tool_calls,
        max_tool_output_bytes,
        max_shell_output_bytes,
    }
}

fn budget_service(
    temp: &tempfile::TempDir,
    steps: Vec<BudgetProviderStep>,
) -> (Service, Arc<BudgetProvider>) {
    let provider = Arc::new(BudgetProvider {
        requests: Arc::new(Mutex::new(Vec::new())),
        steps: Arc::new(steps),
    });
    let service = Service::open(
        service::Config {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        provider.clone(),
    )
    .expect("open service");
    (service, provider)
}

#[tokio::test]
async fn dispatches_tools() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(FakeProvider {
        request_tool: true,
        ..FakeProvider::default()
    });
    let service = Service::open(
        service::Config {
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
    let runtime = Probe::new(&service)
        .completed_turn(&strand.id, &accepted_turn(&response).id)
        .await;
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
    assert!(
        stdout.contains("soul_default"),
        "SANTI_SOUL_ID in shell env: {stdout}"
    );
    assert!(
        stdout.contains(&strand.id),
        "SANTI_STRAND_ID in shell env: {stdout}"
    );
    assert!(
        stdout.contains(&accepted_turn(&response).id),
        "SANTI_TURN_ID in shell env: {stdout}"
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
