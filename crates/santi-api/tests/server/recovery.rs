use super::*;
use santi_core::{message, strand};

#[tokio::test]
async fn send_rejection_locks() {
    let temp = tempfile::tempdir().expect("temp dir");
    let service = Service::open(
        service::Config {
            database: temp.path().join("santi.sqlite").display().to_string(),
            runtime: temp.path().join("runtime").display().to_string(),
            execution: temp.path().join("execution").display().to_string(),
            bind: Some("127.0.0.1:0".to_string()),
            constitution: None,
        },
        Arc::new(BudgetedProvider),
    )
    .expect("open service");
    let strand = service.weave().expect("create strand").strand;

    let error = send_strand_handler(
        State(service),
        Path(strand.id),
        Json(strand::Post {
            content: vec![message::Part::Text {
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
    assert!(body["incident"].as_str().is_some());
    assert_eq!(body["exposure"]["model"], false);
    assert!(
        body.get("reason").is_none(),
        "old error wrapper must not survive"
    );
}

#[tokio::test]
async fn drive_failure_http_recovery() {
    let temp = tempfile::tempdir().expect("temp dir");
    let database = temp.path().join("santi.sqlite");
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
    .expect("open service");
    let strand = service.weave().expect("create strand").strand;
    let conn = Connection::open(&database).expect("open sqlite");
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
        Json(strand::Post {
            content: vec![message::Part::Text {
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
    let receipt =
        receipt_status_handler(State(service.clone()), Path(accepted.receipt.inbox.clone())).await;
    let Json(receipt) = match receipt {
        Ok(receipt) => receipt,
        Err(error) => panic!(
            "receipt query failed with {}: {}",
            error.code(),
            error.message()
        ),
    };
    assert_eq!(receipt.inbox, accepted.receipt.inbox);
    assert_eq!(receipt.state, santi_core::receipt::State::Accepted);

    let health = health_handler(State(service.clone())).await.into_response();
    assert_eq!(health.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(health.into_body(), usize::MAX)
        .await
        .expect("read degraded health");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("health json");
    assert_eq!(body["ok"], false);
    assert_eq!(body["degraded"], true);
    assert_eq!(body["incidents"], 1);
    assert!(body.get("strand").is_none());
    assert!(body.get("inbox").is_none());

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
    assert_eq!(driven.state, santi_core::drive::State::Started);

    let health = health_handler(State(service)).await.into_response();
    assert_eq!(health.status(), StatusCode::OK);
}
