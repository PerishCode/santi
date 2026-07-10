use santi::watch::{json_field, next_sse_frame, parse_sse_frame, render_watch_event, snippet};

#[test]
fn renders_events() {
    assert_eq!(
        render_watch_event("stream_open", r#"{"payload":{"type":"stream_open"}}"#),
        None
    );
    assert_eq!(
        render_watch_event(
            "message_delta",
            r#"{"payload":{"type":"message_delta","text":"chunk"}}"#,
        ),
        None
    );
    assert_eq!(
        render_watch_event(
            "turn_started",
            r#"{"payload":{"type":"turn_started","turn":{"id":"turn_1","trigger_type":"strand_send"}}}"#,
        )
        .as_deref(),
        Some("turn started turn_1 (strand_send)")
    );
    assert_eq!(
        render_watch_event(
            "turn_activity",
            r#"{"payload":{"type":"turn_activity","activity":{"turn_id":"turn_1","state":"running_tool"}}}"#,
        )
        .as_deref(),
        Some("turn turn_1: running_tool")
    );
    assert_eq!(
        render_watch_event(
            "message_completed",
            r#"{"payload":{"type":"message_completed","turn_id":"turn_1","message":{"content_text":"hello\nworld"}}}"#,
        )
        .as_deref(),
        Some("assistant completed turn_1: hello world")
    );
    assert_eq!(
        render_watch_event(
            "tool_result_created",
            r#"{"payload":{"type":"tool_result_created","tool_result":{"tool_call_id":"call_1","error_text":null}}}"#,
        )
        .as_deref(),
        Some("tool result call_1: ok")
    );
    assert_eq!(
        render_watch_event(
            "turn_failed",
            r#"{"payload":{"type":"turn_failed","turn_id":"turn_1","error":{"code":"provider.turn.failed","message":"provider request failed","incident_id":"inc_1"}}}"#,
        )
        .as_deref(),
        Some(
            "turn failed turn_1: provider.turn.failed: provider request failed (incident inc_1)"
        )
    );
    assert_eq!(
        render_watch_event(
            "error_transition",
            r#"{"payload":{"type":"error_transition","transition":{"kind":"opened","incident":{"id":"inc_1","code":"provider.turn.failed","context":{"detail":"secret"}}}}}"#,
        )
        .as_deref(),
        Some("error opened provider.turn.failed (inc_1)")
    );
}

#[test]
fn snippets_text() {
    assert_eq!(snippet("a\n  b\t c", 20), "a b c");
    assert_eq!(snippet("abcdef", 3), "abc…");
}

#[test]
fn parses_sse_frame() {
    let frame = "id: e1\nevent: turn_completed\ndata: {\"payload\":{\"turn_id\":\"t1\"}}\n";
    let (event, data) = parse_sse_frame(frame).expect("frame");
    assert_eq!(event, "turn_completed");
    assert_eq!(data, "{\"payload\":{\"turn_id\":\"t1\"}}");
    assert!(parse_sse_frame(": keep-alive\n").is_none());
}

#[test]
fn reads_json_field() {
    let data = "{\"payload\":{\"turn\":{\"id\":\"t9\"}}}";
    assert_eq!(
        json_field(data, &["payload", "turn", "id"]).as_deref(),
        Some("t9")
    );
    assert_eq!(json_field(data, &["payload", "missing"]), None);
}

#[tokio::test]
async fn frames_across_chunks() {
    use futures_util::stream;

    let chunks: Vec<reqwest::Result<Vec<u8>>> = vec![
        Ok(b"event: turn_started\ndata: {\"payl".to_vec()),
        Ok(b"oad\":{\"turn\":{\"id\":\"t1\"}}}\n\n: ka\n\n".to_vec()),
    ];
    let mut stream = stream::iter(chunks);
    let mut buffer = String::new();
    let (event, data) = next_sse_frame(&mut stream, &mut buffer)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event, "turn_started");
    assert_eq!(
        json_field(&data, &["payload", "turn", "id"]).as_deref(),
        Some("t1")
    );
    assert!(
        next_sse_frame(&mut stream, &mut buffer)
            .await
            .unwrap()
            .is_none()
    );
}
