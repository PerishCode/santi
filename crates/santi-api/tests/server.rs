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
use rusqlite::Connection;
use santi_api::{
    ApiError, ResolveEffectRequest, drive_strand_handler, effect_status_handler, health_handler,
    receipt_status_handler, resolve_effect_handler, send_strand_handler,
};
use santi_core::{
    EffectResolutionOutcome, EffectState, ErrorScope, ErrorSource, IngestOutcome, MessageContent,
    MessageKind, MessagePart, SantiService, SantiServiceConfig, SantiStore, SendStrandRequest,
    ToolCallProvenance, catalog, engine,
};
use santi_provider::{
    ProviderClient, ProviderContextBudget, ProviderEvent, ProviderMetadata, ProviderRequest,
    ProviderStream,
};

struct BudgetedProvider;

struct DriverProvider;

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

#[async_trait]
impl ProviderClient for DriverProvider {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            provider: Arc::from("driver-provider"),
            model: "driver-model".to_string(),
            context_budget: None,
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
    let unavailable = engine().transient(
        catalog::STRAND_DRIVE_FAILED,
        ErrorSource::new("test", "driver"),
        Some(ErrorScope::new("strand", "ss_x")),
        "driver unavailable",
        serde_json::Value::Null,
    );
    assert_eq!(
        ApiError::from_santi(unavailable).status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
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
    assert!(document.contains("/api/v1/strands/{strand_id}/drive"));
    assert!(document.contains("/api/v1/receipts/{inbox_id}"));
    assert!(document.contains("/api/v1/effects/{effect_id}"));
    assert!(document.contains("/api/v1/effects/{effect_id}/resolve"));
    assert!(document.contains("EffectTransitionReason"));
    assert!(document.contains("IngestReceipt"));
}

#[tokio::test]
async fn effect_http_roundtrip() {
    let temp = tempfile::tempdir().expect("temp dir");
    let database_path = temp.path().join("santi.sqlite");
    let store = SantiStore::open(&database_path).expect("open store");
    let strand = store.create_strand().expect("create strand");
    let inbox_id = match store
        .enqueue_inbox(
            &strand.id,
            MessageKind::Text,
            MessageContent::text("run effect"),
        )
        .expect("enqueue")
    {
        IngestOutcome::Accepted { receipt } => receipt.inbox_id,
        IngestOutcome::Rejected { .. } => panic!("unexpected rejection"),
    };
    let turn = store
        .try_start_turn(&strand.id, "strand_send", None)
        .expect("start turn")
        .expect("started turn")
        .turn;
    let (_, effect) = store
        .append_effect_call(
            &turn.id,
            "call_api_effect",
            "shell",
            &serde_json::json!({"command": "printf api"}),
            &ToolCallProvenance::default(),
            Some("shell"),
        )
        .expect("append effect");
    let effect_id = effect.expect("effect").id;
    store
        .begin_effect_dispatch(&effect_id)
        .expect("open dispatch window");
    drop(store);

    let service = SantiService::open(
        SantiServiceConfig {
            database_path: database_path.display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        Arc::new(DriverProvider),
    )
    .expect("restart service");

    let queried = effect_status_handler(State(service.clone()), Path(effect_id.clone())).await;
    let Json(queried) = match queried {
        Ok(queried) => queried,
        Err(error) => panic!(
            "effect query failed with {}: {}",
            error.code(),
            error.message()
        ),
    };
    assert_eq!(queried.effect.state, EffectState::Unknown);
    assert_eq!(queried.receipt_ids, vec![inbox_id]);

    let error = resolve_effect_handler(
        State(service.clone()),
        Path(effect_id.clone()),
        Json(ResolveEffectRequest {
            outcome: EffectResolutionOutcome::Applied,
            evidence: "   ".to_string(),
        }),
    )
    .await
    .expect_err("blank evidence must be rejected");
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);

    let resolved = resolve_effect_handler(
        State(service),
        Path(effect_id),
        Json(ResolveEffectRequest {
            outcome: EffectResolutionOutcome::Applied,
            evidence: "operator found the target marker".to_string(),
        }),
    )
    .await;
    let Json(resolved) = match resolved {
        Ok(resolved) => resolved,
        Err(error) => panic!(
            "effect resolution failed with {}: {}",
            error.code(),
            error.message()
        ),
    };
    assert_eq!(resolved.effect.state, EffectState::ResolvedApplied);
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

#[tokio::test]
async fn drive_failure_http_recovery() {
    let temp = tempfile::tempdir().expect("temp dir");
    let database_path = temp.path().join("santi.sqlite");
    let service = SantiService::open(
        SantiServiceConfig {
            database_path: database_path.display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        Arc::new(DriverProvider),
    )
    .expect("open service");
    let strand = service.create_strand().expect("create strand").strand;
    let conn = Connection::open(&database_path).expect("open sqlite");
    conn.execute_batch(
        r#"
        CREATE TRIGGER force_api_turn_failure
        BEFORE INSERT ON turns
        BEGIN
          SELECT RAISE(ABORT, 'forced api turn failure');
        END;
        "#,
    )
    .expect("install failure trigger");

    let accepted = send_strand_handler(
        State(service.clone()),
        Path(strand.id.clone()),
        Json(SendStrandRequest {
            content: vec![MessagePart::Text {
                text: "x".to_string(),
            }],
        }),
    )
    .await;
    let Json(accepted) = match accepted {
        Ok(accepted) => accepted,
        Err(error) => panic!(
            "durable request should remain accepted, got {}: {}",
            error.code(),
            error.message()
        ),
    };
    let warning = accepted.receipt.warning.expect("canonical warning");
    assert_eq!(warning.code, "runtime.strand.drive_failed");
    assert_eq!(warning.context["accepted_before_failure"], true);
    let receipt = receipt_status_handler(
        State(service.clone()),
        Path(accepted.receipt.inbox_id.clone()),
    )
    .await;
    let Json(receipt) = match receipt {
        Ok(receipt) => receipt,
        Err(error) => panic!(
            "receipt query failed with {}: {}",
            error.code(),
            error.message()
        ),
    };
    assert_eq!(receipt.inbox_id, accepted.receipt.inbox_id);
    assert_eq!(receipt.state, santi_core::ReceiptState::Accepted);

    let health = health_handler(State(service.clone())).await.into_response();
    assert_eq!(health.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(health.into_body(), usize::MAX)
        .await
        .expect("read degraded health");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("health json");
    assert_eq!(body["ok"], false);
    assert_eq!(body["degraded"], true);
    assert_eq!(body["active_drive_incidents"], 1);
    assert!(body.get("strand_id").is_none());
    assert!(body.get("inbox_id").is_none());

    conn.execute_batch("DROP TRIGGER force_api_turn_failure;")
        .expect("remove failure trigger");
    let driven = drive_strand_handler(State(service.clone()), Path(strand.id)).await;
    let Json(driven) = match driven {
        Ok(driven) => driven,
        Err(error) => panic!(
            "explicit drive failed with {}: {}",
            error.code(),
            error.message()
        ),
    };
    assert_eq!(driven.state, santi_core::DriveStrandState::Started);

    let health = health_handler(State(service)).await.into_response();
    assert_eq!(health.status(), StatusCode::OK);
}

fn status(message: &str) -> StatusCode {
    ApiError::from_service(message.to_string()).status()
}
