use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use futures_util::stream;
use santi_api::{ApiError, send_strand_handler};
use santi_core::{MessagePart, SantiService, SantiServiceConfig, SendStrandRequest};
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
    assert_eq!(
        status("strand is blocked: context_over_budget block_id=blk_x: x"),
        StatusCode::LOCKED
    );
    assert_eq!(
        status("strand context is over budget (10 estimated bytes, budget 1)"),
        StatusCode::LOCKED
    );
    assert_eq!(
        status("something unexpected"),
        StatusCode::INTERNAL_SERVER_ERROR
    );
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
    assert_eq!(error.code(), "strand-blocked");
    assert!(error.message().contains("over budget"));
}

fn status(message: &str) -> StatusCode {
    ApiError::from_service(message.to_string()).status()
}
