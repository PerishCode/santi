use async_trait::async_trait;
use futures_util::stream;
use santi_core::{
    ActorType, MessageKind, MessagePart, MessageState, SantiService, SantiServiceConfig,
    SendSessionRequest,
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
    stream_error_after_text: Option<String>,
}

#[async_trait]
impl ProviderClient for FailureProvider {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            provider: Arc::from("fake-provider"),
            model: "fake-model".to_string(),
        }
    }

    async fn stream_response(&self, request: ProviderRequest) -> Result<ProviderStream, String> {
        {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request);
        }
        if let Some(error) = &self.fail_with {
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
        Ok(Box::pin(stream::iter(vec![
            Ok(ProviderEvent::TextDelta("ok".to_string())),
            Ok(ProviderEvent::Completed {
                provider_response_id: Some("fake-response-id".to_string()),
            }),
        ])))
    }
}

#[tokio::test]
async fn records_failed_system() {
    let temp = tempfile::tempdir().expect("temp dir");
    let raw_error = "openai responses request failed: 401 Unauthorized secret detail".to_string();
    let provider = Arc::new(FailureProvider {
        fail_with: Some(raw_error.clone()),
        ..FailureProvider::default()
    });
    let service = open_service(&temp, provider.clone());
    let session = service.create_session().expect("create session").session;
    let response = send_text(&service, &session.session.id, "trigger failure").await;

    let runtime = wait_for_failed_turn(&service, &session.session.id, &response.turn.id).await;
    let failed_turn = runtime
        .turns
        .iter()
        .find(|turn| turn.id == response.turn.id)
        .expect("failed turn");
    assert_eq!(failed_turn.error_text.as_deref(), Some(raw_error.as_str()));

    let system_message = runtime
        .messages
        .iter()
        .find(|message| message.message.message_kind == MessageKind::SantiSystem)
        .expect("santi system message");
    assert_eq!(system_message.message.actor_type, ActorType::System);
    assert_eq!(system_message.message.actor_id, "santi");
    assert_eq!(
        system_message.content_text,
        format!(
            "<santi-system>\nkind: turn_failed\nturn_id: {}\ntrace: log://turn/{}\nsummary: Previous response attempt failed before completion.\n</santi-system>",
            response.turn.id, response.turn.id
        )
    );
    // Use leak markers that cannot appear in a hex turn_id ("401" can, since
    // turn_ids are hex): the raw error's words, not its numeric status code.
    assert!(!system_message.content_text.contains("Unauthorized"));
    assert!(!system_message.content_text.contains("secret detail"));

    let retry = send_text(&service, &session.session.id, "continue after failure").await;
    wait_for_failed_turn(&service, &session.session.id, &retry.turn.id).await;

    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[1]
            .input
            .iter()
            .any(
                |message| as_text(message).is_some_and(|(role, content)| role == "user"
                    && content.contains("<santi-system>")
                    && content.contains("kind: turn_failed"))
            )
    );
}

#[tokio::test]
async fn preserves_aborted_output() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(FailureProvider {
        stream_error_after_text: Some("provider stream aborted".to_string()),
        ..FailureProvider::default()
    });
    let service = open_service(&temp, provider.clone());
    let session = service.create_session().expect("create session").session;
    let response = send_text(&service, &session.session.id, "trigger stream failure").await;

    let runtime = wait_for_failed_turn(&service, &session.session.id, &response.turn.id).await;
    let partial_message = runtime
        .messages
        .iter()
        .find(|message| {
            message.message.actor_type == ActorType::Soul
                && message.message.state == MessageState::Aborted
        })
        .expect("aborted partial assistant message");
    assert_eq!(partial_message.content_text, "partial runtime output");

    let system_message = runtime
        .messages
        .iter()
        .find(|message| message.message.message_kind == MessageKind::SantiSystem)
        .expect("santi system failure message");
    assert!(
        partial_message.relation.session_seq < system_message.relation.session_seq,
        "partial output should precede failure fact"
    );

    let retry = send_text(
        &service,
        &session.session.id,
        "continue with preserved partial",
    )
    .await;
    wait_for_failed_turn(&service, &session.session.id, &retry.turn.id).await;

    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].input.iter().any(|message| {
        as_text(message).is_some_and(|(role, content)| {
            role == "assistant" && content == "partial runtime output"
        })
    }));
    assert!(requests[1].input.iter().any(|message| {
        as_text(message)
            .is_some_and(|(role, content)| role == "user" && content.contains("kind: turn_failed"))
    }));
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

async fn send_text(
    service: &SantiService,
    session_id: &str,
    text: &str,
) -> santi_core::SendSessionAcceptedResponse {
    service
        .send_session(
            session_id,
            SendSessionRequest {
                content: vec![MessagePart::Text {
                    text: text.to_string(),
                }],
                soul_id: None,
            },
        )
        .await
        .expect("send session")
}

async fn wait_for_failed_turn(
    service: &SantiService,
    session_id: &str,
    turn_id: &str,
) -> santi_core::SessionRuntimeSnapshot {
    for _ in 0..50 {
        let runtime = service
            .runtime_snapshot(session_id)
            .expect("runtime snapshot")
            .expect("session runtime");
        let turn_failed = runtime
            .turns
            .iter()
            .any(|turn| turn.id == turn_id && turn.status == santi_core::TurnStatus::Failed);
        let system_recorded = runtime
            .messages
            .iter()
            .any(|message| message.message.message_kind == MessageKind::SantiSystem);
        if turn_failed && system_recorded {
            return runtime;
        }
        sleep(Duration::from_millis(20)).await;
    }
    panic!("turn did not fail");
}
