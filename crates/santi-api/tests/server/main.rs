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
use santi_core::service::{self, Service};
use santi_core::{Invocation, SantiStore, catalog, engine};
use santi_core::{effect, ingest, message, tool};
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
                bytes: 1,
                source: "test".to_string(),
            }),
        }
    }

    async fn stream_response(&self, _request: ProviderRequest) -> Result<ProviderStream, String> {
        Ok(Box::pin(stream::iter(vec![Ok(ProviderEvent::Completed {
            response: None,
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
            response: None,
        })])))
    }
}

#[test]
fn classifies_errors() {
    assert_eq!(status("strand not found"), StatusCode::NOT_FOUND);
    assert_eq!(status("unknown soul: soul_x"), StatusCode::BAD_REQUEST);
    assert_eq!(
        status("downstream digest must be 64 hexadecimal characters"),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        status("downstream request conflicts with an accepted payload"),
        StatusCode::CONFLICT
    );
    assert_eq!(
        ApiError::forbidden("outside zone").status(),
        StatusCode::FORBIDDEN
    );
    let budget = engine().transient(santi_core::Signal {
        descriptor: catalog::CONTEXT_BUDGET_EXCEEDED,
        source: santi_core::Source::new("test", "admission"),
        scope: Some(santi_core::Scope::new("strand", "ss_x")),
        message: "over budget".to_string(),
        context: serde_json::Value::Null,
    });
    assert_eq!(ApiError::from_santi(budget).status(), StatusCode::LOCKED);
    let unavailable = engine().transient(santi_core::Signal {
        descriptor: catalog::STRAND_DRIVE_FAILED,
        source: santi_core::Source::new("test", "driver"),
        scope: Some(santi_core::Scope::new("strand", "ss_x")),
        message: "driver unavailable".to_string(),
        context: serde_json::Value::Null,
    });
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
    assert!(document.contains("/api/v1/strands/{strand}/drive"));
    assert!(document.contains("/api/v1/receipts/{inbox}"));
    assert!(document.contains("/api/v1/effects/{effect_id}"));
    assert!(document.contains("/api/v1/effects/{effect_id}/resolve"));
    assert!(document.contains("effect.Reason"));
    assert!(document.contains("ingest.Receipt"));
    assert!(document.contains("/api/v1/turn-events/stream"));
    assert!(document.contains("event.Batch"));
    assert!(document.contains("request"));
    assert!(document.contains("downstream_bearer"));
    assert!(!document.contains("credential_env"));
}

#[tokio::test]
async fn effect_http_roundtrip() {
    let temp = tempfile::tempdir().expect("temp dir");
    let database_path = temp.path().join("santi.sqlite");
    let store = SantiStore::open(&database_path).expect("open store");
    let strand = store.create_strand().expect("create strand");
    let inbox = match store
        .enqueue_inbox(
            &strand.id,
            message::Kind::Text,
            message::Content::text("run effect"),
        )
        .expect("enqueue")
    {
        ingest::Outcome::Accepted { receipt } => receipt.inbox,
        ingest::Outcome::Rejected { .. } => panic!("unexpected rejection"),
    };
    let turn = store
        .try_start_turn(&strand.id, "strand_send", None)
        .expect("start turn")
        .expect("started turn")
        .turn;
    let (_, effect) = store
        .append_effect_call(
            Invocation {
                turn: &turn.id,
                call: "call_api_effect",
                name: "shell",
                arguments: &serde_json::json!({"command": "printf api"}),
                provenance: &tool::Provenance::default(),
            },
            Some("shell"),
        )
        .expect("append effect");
    let effect_id = effect.expect("effect").id;
    store
        .begin_effect_dispatch(&effect_id)
        .expect("open dispatch window");
    drop(store);

    let service = Service::open(
        service::Config {
            database_path: database_path.display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
            constitution_path: None,
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
    assert_eq!(queried.effect.state, effect::State::Unknown);
    assert_eq!(queried.receipts, vec![inbox]);

    let error = resolve_effect_handler(
        State(service.clone()),
        Path(effect_id.clone()),
        Json(ResolveEffectRequest {
            outcome: effect::Outcome::Applied,
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
            outcome: effect::Outcome::Applied,
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
    assert_eq!(resolved.effect.state, effect::State::ResolvedApplied);
}

mod recovery;

fn status(message: &str) -> StatusCode {
    ApiError::from_service(message.to_string()).status()
}
