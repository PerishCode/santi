use async_trait::async_trait;
use futures_util::stream;
use rusqlite::Connection;
use santi_core::Category;
use santi_core::service::{self, Service};
use santi_core::{message, strand, turn};
use santi_provider::{
    ProviderClient, ProviderEvent, ProviderItem, ProviderMetadata, ProviderRequest, ProviderStream,
};
use std::sync::{Arc, Mutex};
use tokio::time::{Duration, sleep};

#[path = "failure/more.rs"]
mod more;

fn as_text(item: &ProviderItem) -> Option<(&str, &str)> {
    match item {
        ProviderItem::Message { role, content } => Some((role.as_str(), content.as_str())),
        _ => None,
    }
}

#[derive(Clone, Default)]
struct FailureProvider {
    requests: Arc<Mutex<Vec<ProviderRequest>>>,
    fail_with: Option<String>,
    fail_for_requests: Option<usize>,
    stream_error_after_text: Option<String>,
    response_failure: Option<String>,
    response_started: bool,
}

#[async_trait]
impl ProviderClient for FailureProvider {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            provider: Arc::from("fake-provider"),
            model: "fake-model".to_string(),
            context_budget: None,
        }
    }

    async fn stream_response(&self, request: ProviderRequest) -> Result<ProviderStream, String> {
        let attempt = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request);
            requests.len()
        };
        if let Some(error) = &self.fail_with
            && self
                .fail_for_requests
                .is_none_or(|failure_count| attempt <= failure_count)
        {
            return Err(error.clone());
        }
        if let Some(error) = &self.stream_error_after_text {
            return Ok(Box::pin(stream::iter(vec![
                Ok(ProviderEvent::TextDelta(
                    "partial runtime output".to_string(),
                )),
                Err(error.clone()),
            ])));
        }
        if let Some(error) = &self.response_failure {
            return Ok(Box::pin(stream::iter(vec![Ok(ProviderEvent::Failed(
                error.clone(),
            ))])));
        }
        let mut events = Vec::new();
        if self.response_started {
            events.push(Ok(ProviderEvent::ResponseStarted {
                response: Some("fake-response-id".to_string()),
            }));
        }
        events.push(Ok(ProviderEvent::TextDelta("ok".to_string())));
        events.push(Ok(ProviderEvent::Completed {
            response: Some("fake-response-id".to_string()),
        }));
        Ok(Box::pin(stream::iter(events)))
    }
}

#[tokio::test]
async fn aggregates_provider_failures() {
    let temp = tempfile::tempdir().expect("temp dir");
    let raw_error = "openai responses request failed: 401 Unauthorized secret detail".to_string();
    let provider = Arc::new(FailureProvider {
        fail_with: Some(raw_error.clone()),
        ..FailureProvider::default()
    });
    let service = open_service(&temp, provider.clone());
    let mut events = service.subscribe_stream();
    let strand = service.create_strand().expect("create strand").strand;
    let first = send_text(&service, &strand.id, "trigger failure").await;

    let runtime = wait_for_turn(&service, &strand.id, &turn(&first).id, turn::Status::Failed).await;
    let failed_turn = runtime
        .turns
        .iter()
        .find(|candidate| candidate.id == turn(&first).id)
        .expect("failed turn");
    assert_eq!(failed_turn.error.as_deref(), Some(raw_error.as_str()));
    assert_no_failure_projection(&runtime);

    assert_eq!(runtime.errors.len(), 1);
    let incident = &runtime.errors[0];
    assert_eq!(incident.code, "provider.turn.failed");
    assert_eq!(incident.status, santi_core::Status::Active);
    assert_eq!(incident.category, Category::Unavailable);
    assert_eq!(incident.occurrences, 1);
    assert_eq!(incident.revision, 1);
    assert_eq!(incident.first.source.component, "santi-provider");
    assert_eq!(incident.first.source.operation, "turn.request");
    assert_eq!(incident.first.context["turn"], turn(&first).id);
    assert_eq!(incident.first.context["provider"], "fake-provider");
    assert_eq!(incident.first.context["model"], "fake-model");
    assert_eq!(incident.first.context["stage"], "request");
    assert_eq!(incident.first.context["round"], 1);
    assert!(!incident.exposure.model);

    let failed_event = std::iter::from_fn(|| events.try_recv().ok())
        .find_map(|event| match event.payload {
            santi_core::stream::Payload::TurnFailed { turn: held, error }
                if held == turn(&first).id =>
            {
                Some(error)
            }
            _ => None,
        })
        .expect("turn_failed event");
    assert_eq!(failed_event.code, "provider.turn.failed");
    assert_eq!(failed_event.incident.as_deref(), Some(incident.id.as_str()));
    assert_eq!(failed_event.source.operation, "turn.request");

    let retry = send_text(&service, &strand.id, "continue after failure").await;
    let runtime = wait_for_turn(&service, &strand.id, &turn(&retry).id, turn::Status::Failed).await;
    assert_no_failure_projection(&runtime);
    assert_eq!(runtime.errors.len(), 1);
    assert_eq!(runtime.errors[0].id, incident.id);
    assert_eq!(runtime.errors[0].occurrences, 2);
    assert_eq!(runtime.errors[0].revision, 1);
    assert_eq!(runtime.errors[0].latest.context["turn"], turn(&retry).id);
    assert_eq!(
        transition_count(&temp),
        1,
        "repeated failures must not emit lifecycle transitions"
    );

    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].input.iter().all(|item| {
        as_text(item).is_none_or(|(_, content)| {
            !content.contains("kind: turn_failed") && !content.contains("secret detail")
        })
    }));
}

fn assert_no_failure_projection(runtime: &santi_core::stream::Snapshot) {
    assert!(
        runtime
            .messages
            .iter()
            .all(|message| message.message.kind != message::Kind::SantiSystem),
        "provider failure must not append a model-visible system message"
    );
}

fn turn(response: &santi_core::strand::Posted) -> &santi_core::turn::Turn {
    response.turn.as_ref().expect("send should start a turn")
}

fn open_service(temp: &tempfile::TempDir, provider: Arc<FailureProvider>) -> Service {
    Service::open(
        service::Config {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
            constitution_path: None,
        },
        provider,
    )
    .expect("open service")
}

fn transition_count(temp: &tempfile::TempDir) -> i64 {
    Connection::open(temp.path().join("santi.sqlite"))
        .expect("open sqlite")
        .query_row("SELECT COUNT(*) FROM error_transitions", [], |row| {
            row.get(0)
        })
        .expect("transition count")
}

async fn send_text(service: &Service, strand: &str, text: &str) -> santi_core::strand::Posted {
    service
        .send_strand(
            strand,
            strand::Post {
                content: vec![message::Part::Text {
                    text: text.to_string(),
                }],
            },
        )
        .await
        .expect("send strand")
}

async fn wait_for_turn(
    service: &Service,
    strand: &str,
    turn: &str,
    status: turn::Status,
) -> santi_core::stream::Snapshot {
    for _ in 0..100 {
        let runtime = service
            .runtime_snapshot(strand)
            .expect("runtime snapshot")
            .expect("strand runtime");
        if runtime
            .turns
            .iter()
            .any(|held| held.id == turn && held.status == status)
        {
            return runtime;
        }
        sleep(Duration::from_millis(20)).await;
    }
    panic!("turn {turn} did not reach {status:?}");
}

async fn wait_for_aborted_output(
    service: &Service,
    strand: &str,
    turn: &str,
) -> santi_core::stream::Snapshot {
    for _ in 0..100 {
        let runtime = service
            .runtime_snapshot(strand)
            .expect("runtime snapshot")
            .expect("strand runtime");
        let failed = runtime
            .turns
            .iter()
            .any(|held| held.id == turn && held.status == turn::Status::Failed);
        let partial_recorded = runtime.messages.iter().any(|message| {
            message.message.role == message::Role::Soul
                && message.message.state == message::State::Aborted
        });
        if failed && partial_recorded {
            return runtime;
        }
        sleep(Duration::from_millis(20)).await;
    }
    panic!("turn {turn} did not persist aborted output");
}
