use async_trait::async_trait;
use futures_util::stream;
use rusqlite::Connection;
use santi_core::service::{self, Service};
use santi_core::{
    ActorType, ErrorCategory, IncidentStatus, MessageKind, MessagePart, MessageState, ReceiptState,
    SantiStreamPayload, SendStrandRequest, TurnStatus,
};
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
                provider_response_id: Some("fake-response-id".to_string()),
            }));
        }
        events.push(Ok(ProviderEvent::TextDelta("ok".to_string())));
        events.push(Ok(ProviderEvent::Completed {
            provider_response_id: Some("fake-response-id".to_string()),
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

    let runtime = wait_for_turn(&service, &strand.id, &turn(&first).id, TurnStatus::Failed).await;
    let failed_turn = runtime
        .turns
        .iter()
        .find(|candidate| candidate.id == turn(&first).id)
        .expect("failed turn");
    assert_eq!(failed_turn.error_text.as_deref(), Some(raw_error.as_str()));
    assert_no_failure_projection(&runtime);

    assert_eq!(runtime.errors.len(), 1);
    let incident = &runtime.errors[0];
    assert_eq!(incident.code, "provider.turn.failed");
    assert_eq!(incident.status, IncidentStatus::Active);
    assert_eq!(incident.category, ErrorCategory::Unavailable);
    assert_eq!(incident.occurrence_count, 1);
    assert_eq!(incident.revision, 1);
    assert_eq!(incident.source.component, "santi-provider");
    assert_eq!(incident.source.operation, "turn.request");
    assert_eq!(incident.context["turn_id"], turn(&first).id);
    assert_eq!(incident.context["provider"], "fake-provider");
    assert_eq!(incident.context["model"], "fake-model");
    assert_eq!(incident.context["stage"], "request");
    assert_eq!(incident.context["round"], 1);
    assert!(!incident.exposure.model);

    let failed_event = std::iter::from_fn(|| events.try_recv().ok())
        .find_map(|event| match event.payload {
            SantiStreamPayload::TurnFailed { turn_id, error } if turn_id == turn(&first).id => {
                Some(error)
            }
            _ => None,
        })
        .expect("turn_failed event");
    assert_eq!(failed_event.code, "provider.turn.failed");
    assert_eq!(
        failed_event.incident_id.as_deref(),
        Some(incident.id.as_str())
    );
    assert_eq!(failed_event.source.operation, "turn.request");

    let retry = send_text(&service, &strand.id, "continue after failure").await;
    let runtime = wait_for_turn(&service, &strand.id, &turn(&retry).id, TurnStatus::Failed).await;
    assert_no_failure_projection(&runtime);
    assert_eq!(runtime.errors.len(), 1);
    assert_eq!(runtime.errors[0].id, incident.id);
    assert_eq!(runtime.errors[0].occurrence_count, 2);
    assert_eq!(runtime.errors[0].revision, 1);
    assert_eq!(runtime.errors[0].latest_context["turn_id"], turn(&retry).id);
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

fn assert_no_failure_projection(runtime: &santi_core::StrandRuntimeSnapshot) {
    assert!(
        runtime
            .messages
            .iter()
            .all(|message| message.message.message_kind != MessageKind::SantiSystem),
        "provider failure must not append a model-visible system message"
    );
}

fn turn(response: &santi_core::SendStrandAcceptedResponse) -> &santi_core::Turn {
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

async fn send_text(
    service: &Service,
    strand_id: &str,
    text: &str,
) -> santi_core::SendStrandAcceptedResponse {
    service
        .send_strand(
            strand_id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: text.to_string(),
                }],
            },
        )
        .await
        .expect("send strand")
}

async fn wait_for_turn(
    service: &Service,
    strand_id: &str,
    turn_id: &str,
    status: TurnStatus,
) -> santi_core::StrandRuntimeSnapshot {
    for _ in 0..100 {
        let runtime = service
            .runtime_snapshot(strand_id)
            .expect("runtime snapshot")
            .expect("strand runtime");
        if runtime
            .turns
            .iter()
            .any(|turn| turn.id == turn_id && turn.status == status)
        {
            return runtime;
        }
        sleep(Duration::from_millis(20)).await;
    }
    panic!("turn {turn_id} did not reach {status:?}");
}

async fn wait_for_aborted_output(
    service: &Service,
    strand_id: &str,
    turn_id: &str,
) -> santi_core::StrandRuntimeSnapshot {
    for _ in 0..100 {
        let runtime = service
            .runtime_snapshot(strand_id)
            .expect("runtime snapshot")
            .expect("strand runtime");
        let failed = runtime
            .turns
            .iter()
            .any(|turn| turn.id == turn_id && turn.status == TurnStatus::Failed);
        let partial_recorded = runtime.messages.iter().any(|message| {
            message.message.actor_type == ActorType::Soul
                && message.message.state == MessageState::Aborted
        });
        if failed && partial_recorded {
            return runtime;
        }
        sleep(Duration::from_millis(20)).await;
    }
    panic!("turn {turn_id} did not persist aborted output");
}
