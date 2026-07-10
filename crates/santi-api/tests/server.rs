use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    Json,
    body::to_bytes,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use futures_util::stream;
use santi_api::{ApiError, send_strand_handler};
use santi_core::{
    ErrorScope, ErrorSource, MessagePart, SantiService, SantiServiceConfig, SendStrandRequest,
    catalog, engine,
};
use santi_provider::{
    ProviderClient, ProviderContextBudget, ProviderEvent, ProviderMetadata, ProviderRequest,
    ProviderStream,
};

struct BudgetedProvider;

#[async_trait]
impl ProviderClient for BudgetedProvider {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            provider: Arc::from("budgeted-provider"),
            model: "budgeted-model".to_string(),
            context_budget: Some(ProviderContextBudget {
                input_budget_bytes: 1,
                source: "test".to_string(),
            }),
        }
    }

    async fn stream_response(&self, _request: ProviderRequest) -> Result<ProviderStream, String> {
        Ok(Box::pin(stream::iter(vec![Ok(ProviderEvent::Completed {
            provider_response_id: None,
        })])))
    }
}

#[test]
fn classifies_errors() {
    assert_eq!(status("strand not found"), StatusCode::NOT_FOUND);
    assert_eq!(status("unknown soul: soul_x"), StatusCode::BAD_REQUEST);
    let budget = engine().transient(
        catalog::CONTEXT_BUDGET_EXCEEDED,
        ErrorSource::new("test", "admission"),
        Some(ErrorScope::new("strand", "ss_x")),
        "over budget",
        serde_json::Value::Null,
    );
    assert_eq!(ApiError::from_santi(budget).status(), StatusCode::LOCKED);
    assert_eq!(
        status("something unexpected"),
        StatusCode::INTERNAL_SERVER_ERROR
    );
}

#[test]
fn openapi_lists_error_surfaces() {
    let document = santi_api::export_openapi_json().expect("export openapi");
    assert!(document.contains("/api/v1/errors/{scope_kind}/{scope_id}"));
    assert!(document.contains("/api/v1/errors/events"));
}

#[tokio::test]
async fn send_rejection_locks() {
    let temp = tempfile::tempdir().expect("temp dir");
    let service = SantiService::open(
        SantiServiceConfig {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        Arc::new(BudgetedProvider),
    )
    .expect("open service");
    let strand = service.create_strand().expect("create strand").strand;

    let error = send_strand_handler(
        State(service),
        Path(strand.id),
        Json(SendStrandRequest {
            content: vec![MessagePart::Text {
                text: "this exceeds the tiny budget".to_string(),
            }],
        }),
    )
    .await
    .expect_err("send should be rejected");

    assert_eq!(error.status(), StatusCode::LOCKED);
    assert_eq!(error.code(), "context.budget.exceeded");
    assert!(error.message().contains("over budget"));
    let response = error.into_response();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("error json");
    assert_eq!(body["code"], "context.budget.exceeded");
    assert!(body["incident_id"].as_str().is_some());
    assert_eq!(body["exposure"]["model"], false);
    assert!(
        body.get("reason").is_none(),
        "old error wrapper must not survive"
    );
}

fn status(message: &str) -> StatusCode {
    ApiError::from_service(message.to_string()).status()
}
