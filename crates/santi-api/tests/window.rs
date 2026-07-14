use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    body::{Bytes, to_bytes},
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
};
use futures_util::stream;
use santi_api::{TranscriptQuery, window_send_handler, window_transcript_handler};
use santi_core::service::{self, Service};
use santi_provider::{
    ProviderClient, ProviderEvent, ProviderMetadata, ProviderRequest, ProviderStream,
};

struct EchoProvider;

#[async_trait]
impl ProviderClient for EchoProvider {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            provider: Arc::from("echo-provider"),
            model: "echo-model".to_string(),
            context_budget: None,
        }
    }

    async fn stream_response(&self, _request: ProviderRequest) -> Result<ProviderStream, String> {
        Ok(Box::pin(stream::iter(vec![
            Ok(ProviderEvent::TextDelta("ack".to_string())),
            Ok(ProviderEvent::Completed {
                provider_response_id: None,
            }),
        ])))
    }
}

fn open_service(temp: &tempfile::TempDir) -> Service {
    Service::open(
        service::Config {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        Arc::new(EchoProvider),
    )
    .expect("open service")
}

fn identified(uid: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-authentik-uid", HeaderValue::from_str(uid).unwrap());
    headers
}

fn body(content: serde_json::Value) -> Bytes {
    Bytes::from(content.to_string())
}

async fn response_of(result: impl IntoResponse) -> (StatusCode, serde_json::Value, HeaderMap) {
    let response = result.into_response();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json, headers)
}

#[tokio::test]
async fn missing_identity_is_unauthorized() {
    let temp = tempfile::tempdir().expect("temp dir");
    let service = open_service(&temp);
    let result = window_send_handler(
        State(service.clone()),
        HeaderMap::new(),
        body(serde_json::json!({ "content": "hi", "client_message_id": "k" })),
    )
    .await;
    let (status, error, _) = response_of(result.expect_err("must reject")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(error["code"], "window.identity.missing");
}

#[tokio::test]
async fn non_string_content_is_window_invalid() {
    let temp = tempfile::tempdir().expect("temp dir");
    let service = open_service(&temp);
    let result = window_send_handler(
        State(service.clone()),
        identified("uid-a"),
        body(serde_json::json!({ "content": 123, "client_message_id": "k" })),
    )
    .await;
    let (status, error, _) = response_of(result.expect_err("must reject")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["code"], "window.content.invalid");
}

#[tokio::test]
async fn status_mapping_conflict_oversize_rate() {
    let temp = tempfile::tempdir().expect("temp dir");
    let service = open_service(&temp);

    let accepted = window_send_handler(
        State(service.clone()),
        identified("uid-b"),
        body(serde_json::json!({ "content": "one", "client_message_id": "k1" })),
    )
    .await
    .map_err(|error| error.code().to_string())
    .expect("accepted");
    assert_eq!(accepted.0.status, "accepted");
    assert!(accepted.0.cursor.is_none());

    let conflict = window_send_handler(
        State(service.clone()),
        identified("uid-b"),
        body(serde_json::json!({ "content": "two", "client_message_id": "k1" })),
    )
    .await
    .expect_err("conflict");
    let (status, error, _) = response_of(conflict).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["code"], "window.message.conflict");

    let oversize = window_send_handler(
        State(service.clone()),
        identified("uid-b"),
        body(serde_json::json!({
            "content": "x".repeat(16 * 1024 + 1),
            "client_message_id": "k2"
        })),
    )
    .await
    .expect_err("oversize");
    let (status, error, _) = response_of(oversize).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(error["code"], "window.content.oversize");

    for index in 0..4 {
        let _ = window_send_handler(
            State(service.clone()),
            identified("uid-b"),
            body(serde_json::json!({
                "content": "burst",
                "client_message_id": format!("burst-{index}")
            })),
        )
        .await
        .map_err(|error| error.code().to_string())
        .expect("burst accepted");
    }
    let limited = window_send_handler(
        State(service.clone()),
        identified("uid-b"),
        body(serde_json::json!({ "content": "burst", "client_message_id": "burst-4" })),
    )
    .await
    .expect_err("rate limited");
    let (status, error, headers) = response_of(limited).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(error["code"], "window.rate.limited");
    let retry_after = headers
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .expect("retry-after header");
    assert!(retry_after >= 1);
}

fn transcript_query(since: i64) -> Query<TranscriptQuery> {
    Query(serde_json::from_value(serde_json::json!({ "since": since })).expect("query"))
}

#[tokio::test]
async fn transcript_is_bound_to_the_caller() {
    let temp = tempfile::tempdir().expect("temp dir");
    let service = open_service(&temp);
    let _ = window_send_handler(
        State(service.clone()),
        identified("uid-owner"),
        body(serde_json::json!({ "content": "mine", "client_message_id": "k" })),
    )
    .await
    .map_err(|error| error.code().to_string())
    .expect("accepted");
    for _ in 0..100 {
        let owner = window_transcript_handler(
            State(service.clone()),
            identified("uid-owner"),
            transcript_query(0),
        )
        .await
        .map_err(|error| error.code().to_string())
        .expect("owner transcript");
        if !owner.0.entries.is_empty() {
            assert!(owner.0.entries.iter().any(|entry| entry.text == "mine"));
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let stranger = window_transcript_handler(
        State(service.clone()),
        identified("uid-stranger"),
        transcript_query(0),
    )
    .await
    .map_err(|error| error.code().to_string())
    .expect("stranger transcript");
    assert!(
        stranger.0.empty,
        "another identity sees only its own empty transcript"
    );
    assert!(stranger.0.entries.is_empty());
    assert_ne!(stranger.0.participant, {
        let owner = window_transcript_handler(
            State(service.clone()),
            identified("uid-owner"),
            transcript_query(0),
        )
        .await
        .map_err(|error| error.code().to_string())
        .expect("owner transcript");
        owner.0.participant.clone()
    });

    let missing = window_transcript_handler(
        State(service.clone()),
        HeaderMap::new(),
        transcript_query(0),
    )
    .await
    .expect_err("missing identity");
    let (status, error, _) = response_of(missing).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(error["code"], "window.identity.missing");
}
