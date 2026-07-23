use super::*;

#[tokio::test]
async fn drain_preserves_provenance() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(GatedFirstProvider::new());
    let service = Service::open(
        service::Config {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
            constitution_path: None,
        },
        provider.clone(),
    )
    .expect("open service");

    let strand = service.create_strand().expect("create strand").strand;
    let first = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "first request".to_string(),
                }],
            },
        )
        .await
        .expect("send first request");
    let first_message_id = first
        .message
        .as_ref()
        .expect("first send drains immediately")
        .message
        .id
        .clone();

    provider.wait_for_first_request().await;

    let second = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "second request".to_string(),
                }],
            },
        )
        .await
        .expect("send second request while first runs");
    assert!(second.message.is_none());

    provider.release_first_request();
    let runtime = Probe::new(&service).completed_count(&strand.id, 2).await;

    let second_message = runtime
        .messages
        .iter()
        .find(|message| message.text == "second request")
        .expect("second message drained after first turn");
    let second_event = runtime
        .events
        .iter()
        .find(|event| event.message == second_message.message.id)
        .expect("second message drain event");

    assert_eq!(second_event.payload["kind"], "inbox_drain");
    assert_eq!(second_event.payload["message"], second_message.message.id);
    assert_eq!(
        second_event.payload["drained_at"],
        second_message.message.created
    );
    assert_eq!(second_event.created, second_message.message.created);
    assert_eq!(
        second_event.payload["source"]["type"], "strand_send",
        "direct sends should carry caller/source shape"
    );
    assert_eq!(second_event.payload["source"]["ref"], strand.id);

    let follow_on_turn = runtime
        .turns
        .iter()
        .find(|turn| turn.id != accepted_turn(&first).id)
        .expect("follow-on turn");
    assert_eq!(
        second_event.payload["committing_turn_id"], follow_on_turn.id,
        "the drain event should name the turn that committed the pending request"
    );

    let enqueued_at = second_event.payload["enqueued_at"]
        .as_str()
        .expect("enqueued_at string");
    assert!(
        enqueued_at <= second_message.message.created.as_str(),
        "enqueue time should not be later than drain/message time"
    );

    let first_event = runtime
        .events
        .iter()
        .find(|event| event.message == first_message_id)
        .expect("first message drain event");
    assert_ne!(
        first_event.payload["inbox"], second_event.payload["inbox"],
        "each inbound request should keep its own inbox id provenance"
    );
}
