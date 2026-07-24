use santi_model::{message, stream, turn};

#[test]
fn nested_payload_wire_shape() {
    let payload = stream::Payload::Message(message::Beat::Delta {
        message: "m1".to_string(),
        turn: "t1".to_string(),
        role: message::Role::Soul,
        text: "chunk".to_string(),
    });
    let value = serde_json::to_value(&payload).expect("serialize");
    assert_eq!(value["type"], "message");
    assert_eq!(value["beat"], "delta");
    assert_eq!(value["turn"], "t1");
    assert_eq!(value["text"], "chunk");
    let back: stream::Payload = serde_json::from_value(value).expect("deserialize");
    assert!(matches!(
        back,
        stream::Payload::Message(message::Beat::Delta { .. })
    ));

    let payload = stream::Payload::Turn(turn::Beat::Completed {
        turn: "t1".to_string(),
        label: None,
        text: None,
    });
    let value = serde_json::to_value(&payload).expect("serialize");
    assert_eq!(value["type"], "turn");
    assert_eq!(value["beat"], "completed");
    assert_eq!(value["turn"], "t1");
    assert!(value.get("label").is_none());

    let value = serde_json::to_value(&stream::Payload::Open).expect("serialize");
    assert_eq!(value["type"], "open");
}
