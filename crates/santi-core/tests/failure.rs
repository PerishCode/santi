use async_trait::async_trait;
use futures_util::stream;
use rusqlite::Connection;
use santi_core::{
    ActorType, ErrorCategory, IncidentStatus, MessageKind, MessagePart, MessageState, SantiService,
    SantiServiceConfig, SantiStreamPayload, SendStrandRequest, TurnStatus,
};
use santi_provider::{
    ProviderClient, ProviderEvent, ProviderItem, ProviderMetadata, ProviderRequest, ProviderStream,
};
use std::sync::{Arc, Mutex};
use tokio::time::{Duration, sleep};

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
        Ok(Box::pin(stream::iter(vec![
            Ok(ProviderEvent::TextDelta("ok".to_string())),
            Ok(ProviderEvent::Completed {
                provider_response_id: Some("fake-response-id".to_string()),
            }),
        ])))
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

#[tokio::test]
async fn preserves_aborted_output() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(FailureProvider {
        stream_error_after_text: Some("provider stream aborted".to_string()),
        ..FailureProvider::default()
    });
    let service = open_service(&temp, provider.clone());
    let strand = service.create_strand().expect("create strand").strand;
    let response = send_text(&service, &strand.id, "trigger stream failure").await;

    let runtime = wait_for_aborted_output(&service, &strand.id, &turn(&response).id).await;
    let partial_message = runtime
        .messages
        .iter()
        .find(|message| {
            message.message.actor_type == ActorType::Soul
                && message.message.state == MessageState::Aborted
        })
        .expect("aborted partial assistant message");
    assert_eq!(partial_message.content_text, "partial runtime output");
    assert_no_failure_projection(&runtime);
    assert_eq!(runtime.errors.len(), 1);
    assert_eq!(runtime.errors[0].source.operation, "turn.stream");
    assert_eq!(runtime.errors[0].context["stage"], "stream");

    let retry = send_text(&service, &strand.id, "continue with preserved partial").await;
    wait_for_turn(&service, &strand.id, &turn(&retry).id, TurnStatus::Failed).await;

    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].input.iter().any(|message| {
        as_text(message).is_some_and(|(role, content)| {
            role == "assistant" && content == "partial runtime output"
        })
    }));
    assert!(requests[1].input.iter().all(|message| {
        as_text(message).is_none_or(|(_, content)| !content.contains("kind: turn_failed"))
    }));
}

#[tokio::test]
async fn classifies_response_failure() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(FailureProvider {
        response_failure: Some("provider rejected response".to_string()),
        ..FailureProvider::default()
    });
    let service = open_service(&temp, provider);
    let strand = service.create_strand().expect("create strand").strand;
    let response = send_text(&service, &strand.id, "trigger response failure").await;

    let runtime = wait_for_turn(
        &service,
        &strand.id,
        &turn(&response).id,
        TurnStatus::Failed,
    )
    .await;
    assert_no_failure_projection(&runtime);
    assert_eq!(runtime.errors.len(), 1);
    assert_eq!(runtime.errors[0].source.operation, "turn.response");
    assert_eq!(runtime.errors[0].context["stage"], "response");
}

#[tokio::test]
async fn success_resolves_incident() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(FailureProvider {
        fail_with: Some("temporary provider outage".to_string()),
        fail_for_requests: Some(1),
        ..FailureProvider::default()
    });
    let service = open_service(&temp, provider.clone());
    let strand = service.create_strand().expect("create strand").strand;
    let failed = send_text(&service, &strand.id, "first attempt").await;
    let before = wait_for_turn(&service, &strand.id, &turn(&failed).id, TurnStatus::Failed).await;
    let incident_id = before.errors[0].id.clone();

    let recovered = send_text(&service, &strand.id, "retry after recovery").await;
    let after = wait_for_turn(
        &service,
        &strand.id,
        &turn(&recovered).id,
        TurnStatus::Completed,
    )
    .await;

    assert_no_failure_projection(&after);
    assert_eq!(provider.requests.lock().unwrap().len(), 2);
    assert_eq!(after.errors.len(), 1);
    let incident = &after.errors[0];
    assert_eq!(incident.id, incident_id);
    assert_eq!(incident.status, IncidentStatus::Resolved);
    assert_eq!(incident.occurrence_count, 1);
    assert_eq!(incident.revision, 2);
    assert_eq!(
        incident.resolved_by.as_deref(),
        Some("provider.turn_succeeded")
    );
    assert_eq!(incident.latest_context["turn_id"], turn(&recovered).id);
    assert_eq!(incident.latest_context["provider"], "fake-provider");
    assert_eq!(incident.latest_context["model"], "fake-model");
    assert_eq!(
        transition_count(&temp),
        2,
        "only open and resolve are lifecycle transitions"
    );
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

fn open_service(temp: &tempfile::TempDir, provider: Arc<FailureProvider>) -> SantiService {
    SantiService::open(
        SantiServiceConfig {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
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
    service: &SantiService,
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
    service: &SantiService,
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
    service: &SantiService,
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
