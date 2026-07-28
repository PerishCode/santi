use super::*;
use santi_core::{message, strand};

#[tokio::test]
async fn locks() {
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
    .await
    .expect("open service");
    let strand = service.weave().await.expect("create strand").strand;

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
