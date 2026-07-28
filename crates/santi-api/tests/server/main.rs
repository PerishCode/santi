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
use santi_api::{
    ApiError, ResolveEffectRequest, effect_status_handler, resolve_effect_handler,
    send_strand_handler,
};
use santi_core::service::{self, Service};
use santi_core::{Ruled, Store, budget, drive, engine};
use santi_core::{effect, message};
use santi_estate::{CallDraft, DrainDraft, EffectDraft, InboxDraft, Opening, StrandDraft};
use santi_provider::{Cap, Event, Metadata, Provider, Request, Streaming};

struct BudgetedProvider;

struct DriverProvider;

#[async_trait]
impl Provider for BudgetedProvider {
    fn metadata(&self) -> Metadata {
        Metadata {
            provider: Arc::from("budgeted-provider"),
            model: "budgeted-model".to_string(),
            budget: Some(Cap {
                bytes: 1,
                source: "test".to_string(),
            }),
        }
    }

    async fn stream(&self, _request: Request) -> Result<Streaming, String> {
        Ok(Box::pin(stream::iter(vec![Ok(Event::Completed {
            response: None,
        })])))
    }
}

#[async_trait]
impl Provider for DriverProvider {
    fn metadata(&self) -> Metadata {
        Metadata {
            provider: Arc::from("driver-provider"),
            model: "driver-model".to_string(),
            budget: None,
        }
    }

    async fn stream(&self, _request: Request) -> Result<Streaming, String> {
        Ok(Box::pin(stream::iter(vec![Ok(Event::Completed {
            response: None,
        })])))
    }
}

mod errors {
    use super::*;

    #[test]
    fn classifies() {
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
            status("webhook secretary conflicts with an existing subscription"),
            StatusCode::CONFLICT
        );
        assert_eq!(
            status("webhook delivery conflicts with an accepted payload"),
            StatusCode::CONFLICT
        );
        assert_eq!(
            status("launchd did not accept job job_x: unavailable"),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            ApiError::forbidden("outside zone").status(),
            StatusCode::FORBIDDEN
        );
        let budget = engine().transient(santi_core::Signal {
            descriptor: budget::Error::Context.descriptor(),
            source: santi_core::Source::new("test", "admission"),
            scope: Some(santi_core::Scope::new("strand", "ss_x")),
            message: "over budget".to_string(),
            context: serde_json::Value::Null,
        });
        assert_eq!(ApiError::from_santi(budget).status(), StatusCode::LOCKED);
        let unavailable = engine().transient(santi_core::Signal {
            descriptor: drive::Error::Failed.descriptor(),
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
}

mod openapi {
    #[test]
    fn lists() {
        let document = santi_api::export_openapi_json().expect("export openapi");
        assert!(document.contains("/api/v1/errors/{scope_kind}/{scope_id}"));
        assert!(document.contains("/api/v1/errors/events"));
        assert!(document.contains("/api/v1/strands/{strand}/drive"));
        assert!(document.contains("/api/v1/receipts/{inbox}"));
        assert!(document.contains("/api/v1/effects/{effect}"));
        assert!(document.contains("/api/v1/effects/{effect}/resolve"));
        assert!(document.contains("/api/v1/effects/{effect}/trace"));
        assert!(document.contains("/api/v1/jobs/{job}/logs"));
        assert!(document.contains("job.Accepted"));
        assert!(document.contains("trace.Record"));
        assert!(document.contains("ingest.Receipt"));
        assert!(document.contains("/api/v1/turn-events/stream"));
        assert!(document.contains("/api/v1/turns/{turn}/stop"));
        assert!(document.contains("turn.Stop"));
        assert!(document.contains("event.Batch"));
        assert!(document.contains("request"));
        assert!(document.contains("downstream_bearer"));
        assert!(!document.contains("credential_env"));
    }
}

mod effects {
    use super::*;

    #[tokio::test]
    async fn roundtrip() {
        let temp = tempfile::tempdir().expect("temp dir");
        let database = temp.path().join("santi.sqlite");
        let store = Store::open(&database).await.expect("open store");
        store
            .seed(santi_core::GENESIS, &santi_core::now())
            .await
            .expect("seed");
        let strand = store
            .create_strand(StrandDraft {
                tag: "ss_effect",
                soul: santi_core::GENESIS,
                label: None,
                parent: None,
                fork: None,
                created: &santi_core::now(),
            })
            .await
            .expect("create strand");
        let inbox = "inbox_effect";
        store
            .accept_inbox(
                InboxDraft {
                    tag: inbox,
                    strand: &strand.id,
                    kind: message::Kind::Text,
                    content: &message::Content::text("run effect"),
                    source: None,
                    created: &santi_core::now(),
                },
                500,
            )
            .await
            .expect("enqueue");
        let Opening::Started(started) = store
            .drain_turn(DrainDraft {
                turn: "turn_effect",
                strand: &strand.id,
                trigger: santi_core::turn::Trigger::StrandSend,
                source: None,
                actor: santi_core::SYSTEM,
                created: &santi_core::now(),
            })
            .await
            .expect("start turn")
        else {
            panic!("turn did not start");
        };
        let (_, effect) = store
            .prepare_invocation(
                CallDraft {
                    tag: "call_api_effect",
                    turn: &started.turn.id,
                    tool: "shell",
                    arguments: &serde_json::json!({"command": "printf api"}),
                    created: &santi_core::now(),
                },
                Some(EffectDraft {
                    tag: "effect_api",
                    turn: &started.turn.id,
                    call: Some("call_api_effect"),
                    kind: "shell",
                    metadata: None,
                    created: &santi_core::now(),
                }),
            )
            .await
            .expect("append effect");
        let effect = effect.expect("effect").id;
        store
            .dispatch_effect(&effect, &santi_core::now())
            .await
            .expect("open dispatch window");
        drop(store);

        let service = Service::open(
            service::Config {
                database: database.display().to_string(),
                runtime: temp.path().join("runtime").display().to_string(),
                execution: temp.path().join("execution").display().to_string(),
                bind: Some("127.0.0.1:0".to_string()),
                constitution: None,
            },
            Arc::new(DriverProvider),
        )
        .await
        .expect("restart service");

        let queried = effect_status_handler(State(service.clone()), Path(effect.clone())).await;
        let Json(queried) = match queried {
            Ok(queried) => queried,
            Err(error) => panic!(
                "effect query failed with {}: {}",
                error.code(),
                error.message()
            ),
        };
        assert_eq!(queried.effect.state, effect::State::Unknown);
        assert_eq!(queried.receipts, vec![inbox.to_string()]);

        let error = resolve_effect_handler(
            State(service.clone()),
            Path(effect.clone()),
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
            Path(effect),
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
        assert_eq!(
            resolved.effect.state,
            effect::State::Settled(effect::Outcome::Applied)
        );
    }
}

mod jobs;
mod recovery;

fn status(message: &str) -> StatusCode {
    ApiError::from_service(message.to_string()).status()
}
