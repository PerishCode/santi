use super::support::*;

#[tokio::test]
async fn emits_turn_completed() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(FakeProvider::default());
    let service = SantiService::open(
        SantiServiceConfig {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
        },
        provider.clone(),
    )
    .expect("open service");

    // Subscribe before sending so no lifecycle event is missed.
    let mut events = service.subscribe_stream();
    let strand = service.create_strand().expect("create strand").strand;
    let response = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "say hi".to_string(),
                }],
            },
        )
        .await
        .expect("send strand");

    // The CLI `--watch` idle check relies on a terminal turn event carrying the
    // same turn_id the send landed on. Drain the stream until it arrives.
    let turn_id = response.turn.id.clone();
    let completed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await.expect("stream event").payload {
                santi_core::SantiStreamPayload::TurnCompleted { turn_id } => break turn_id,
                _ => continue,
            }
        }
    })
    .await
    .expect("turn_completed within timeout");
    assert_eq!(completed, turn_id);
}
