use super::support::*;
use santi_core::service::{self, Service};
use santi_core::{effect, message, strand};

mod more;

#[derive(Clone)]
enum BudgetProviderStep {
    Calls { count: usize, output_bytes: usize },
    Fail(String),
    Complete,
}

#[derive(Clone)]
struct BudgetProvider {
    requests: Arc<Mutex<Vec<Request>>>,
    steps: Arc<Vec<BudgetProviderStep>>,
}

#[async_trait]
impl Provider for BudgetProvider {
    fn metadata(&self) -> Metadata {
        Metadata {
            provider: Arc::from("budget-provider"),
            model: "budget-model".to_string(),
            budget: None,
        }
    }

    async fn stream(&self, request: Request) -> Result<Streaming, String> {
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
                Ok(Box::pin(stream::iter(vec![Ok(Event::Completed {
                    response: Some(format!("resp_{index}")),
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

fn tool_events(index: usize, count: usize, output_bytes: usize) -> Vec<Result<Event, String>> {
    let mut events = Vec::new();
    for call_index in 0..count {
        let call = format!("call_{index}_{call_index}");
        let response = format!("resp_{index}");
        let command = output_command(output_bytes);
        let arguments = json!({"command": command});
        let raw = arguments.to_string();
        events.push(Ok(Event::Called(Call {
            response: response.clone(),
            mark: Some(format!("item_{call}")),
            item: json!({
                "type": "function_call",
                "id": format!("item_{call}"),
                "call_id": call,
                "name": "shell",
                "arguments": raw,
            }),
            call,
            name: "shell".to_string(),
            raw,
            arguments,
        })));
    }
    events.push(Ok(Event::Completed {
        response: Some(format!("resp_{index}")),
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
    rounds: usize,
    calls: usize,
    output: usize,
    shell: usize,
) -> santi_core::budget::Execution {
    santi_core::budget::Execution {
        profile: "test_budget".to_string(),
        rounds,
        calls,
        output,
        shell,
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
            database: temp.path().join("santi.sqlite").display().to_string(),
            runtime: temp.path().join("runtime").display().to_string(),
            execution: temp.path().join("execution").display().to_string(),
            bind: Some("127.0.0.1:0".to_string()),
            constitution: None,
        },
        provider.clone(),
    )
    .expect("open service");
    (service, provider)
}

#[tokio::test]
async fn dispatches() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(FakeProvider {
        request_tool: true,
        ..FakeProvider::default()
    });
    let service = Service::open(
        service::Config {
            database: temp.path().join("santi.sqlite").display().to_string(),
            runtime: temp.path().join("runtime").display().to_string(),
            execution: temp.path().join("execution").display().to_string(),
            bind: Some("127.0.0.1:0".to_string()),
            constitution: None,
        },
        provider.clone(),
    )
    .expect("open service");

    let strand = service.weave().expect("create strand").strand;
    let response = service
        .send(
            &strand.id,
            strand::Post {
                content: vec![message::Part::Text {
                    text: "run tool".to_string(),
                }],
            },
        )
        .await
        .expect("send strand");

    assert_eq!(
        accepted_turn(&response).status,
        santi_core::turn::Status::Running
    );
    let runtime = Probe::new(&service)
        .completed_turn(&strand.id, &accepted_turn(&response).id)
        .await;
    assert!(
        runtime
            .messages
            .iter()
            .any(|message| message.text == "hi from runtime")
    );
    assert_eq!(runtime.calls.len(), 1);
    assert_eq!(runtime.calls[0].tool, "shell");
    assert_eq!(runtime.results.len(), 1);
    assert!(runtime.results[0].error.is_none());
    let output = runtime.results[0].output.as_ref().expect("tool output");
    let stdout = output
        .get("stdout")
        .and_then(|value| value.as_str())
        .expect("shell stdout");
    let strandhome = Path::new("runtime")
        .join("strands")
        .join(&strand.id)
        .join("memory");
    assert!(stdout.contains(&strandhome.display().to_string()));
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
    assert!(Path::new(cwd).ends_with(&strandhome));

    assert_eq!(runtime.effects.len(), 1);
    let effect = &runtime.effects[0];
    assert_eq!(effect.call.as_deref(), Some("call_shell"));
    assert_eq!(effect.kind, "shell");
    assert_eq!(
        effect.state,
        effect::State::Settled(effect::Outcome::Applied)
    );
    assert_eq!(
        effect.result.as_deref(),
        Some(runtime.results[0].id.as_str())
    );
    let effect = service
        .effect(&effect.id)
        .expect("query effect")
        .expect("shell effect");
    let mut trail = Vec::new();
    for _ in 0..100 {
        trail = service.trail(&effect.effect.id).expect("query trail");
        if trail.len() >= 3 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let shifts = trail
        .iter()
        .map(|record| {
            let held = |key: &str| {
                record
                    .tags
                    .iter()
                    .find(|tag| tag.key == key)
                    .map(|tag| tag.value.as_str())
                    .expect("shift tag")
            };
            (held("state"), held("reason"))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        shifts,
        vec![
            ("prepared", "intent_persisted"),
            ("dispatching", "dispatch_window_opened"),
            ("settled_applied", "result_persisted"),
        ]
    );
    assert_eq!(effect.receipts, vec![response.receipt.inbox.clone()]);
    let receipt = service
        .receipt(&response.receipt.inbox)
        .expect("query receipt")
        .expect("receipt");
    assert_eq!(receipt.effects.len(), 1);
    assert_eq!(receipt.effects[0].id, effect.effect.id);
    assert_eq!(
        receipt.effects[0].state,
        effect::State::Settled(effect::Outcome::Applied)
    );

    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].previous.is_none());
    assert!(
        requests[1]
            .input
            .iter()
            .any(|item| matches!(item, Item::Call { .. }))
    );
    assert!(
        requests[1]
            .input
            .iter()
            .any(|item| matches!(item, Item::Output { .. }))
    );
}
