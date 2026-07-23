use santi_provider::{OpenAIProviderConfig, ProviderEvent, ProviderStreamTrace};
use serde_json::Value;

#[tokio::test]
async fn optional_params_sent() {
    let body = capture_body(OpenAIProviderConfig {
        api_key: "test-key".to_string(),
        model: "gpt-5.5".to_string(),
        base_url: String::new(),
        reasoning_effort: Some("medium".to_string()),
        reasoning_summary: Some("auto".to_string()),
        max_output_tokens: Some(4096),
        bytes: None,
    })
    .await;

    assert_eq!(body["reasoning"]["effort"], "medium");
    assert_eq!(body["reasoning"]["summary"], "auto");
    assert_eq!(body["max_output_tokens"], 4096);
    assert_eq!(body["stream"], true);
    assert_eq!(body["store"], false);
    assert_eq!(body["stream_options"]["include_obfuscation"], false);
    assert_eq!(body["instructions"], "system guidance");
    assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
    assert_eq!(body["tools"][0]["name"], "shell");
}

#[tokio::test]
async fn optional_params_omitted() {
    let body = capture_body(OpenAIProviderConfig {
        api_key: "test-key".to_string(),
        model: "gpt-4.1".to_string(),
        base_url: String::new(),
        reasoning_effort: None,
        reasoning_summary: None,
        max_output_tokens: None,
        bytes: None,
    })
    .await;

    assert!(body.get("reasoning").is_none());
    assert!(body.get("max_output_tokens").is_none());
    assert_eq!(body["store"], false);
}

#[tokio::test]
async fn plain_requests_unstored() {
    let body = capture_body_without_tools(OpenAIProviderConfig {
        api_key: "test-key".to_string(),
        model: "gpt-4.1".to_string(),
        base_url: String::new(),
        reasoning_effort: None,
        reasoning_summary: None,
        max_output_tokens: None,
        bytes: None,
    })
    .await;

    assert_eq!(body["store"], false);
}

#[tokio::test]
async fn parses_call_response_id() {
    let events = capture_events(vec![
        r#"data: {"type":"response.created","response":{"id":"resp_tool"}}"#,
        r#"data: {"type":"response.output_item.done","item":{"type":"function_call","id":"item_shell","call_id":"call_shell","name":"shell","arguments":"{\"cmd\":\"pwd\"}"}}"#,
    ])
    .await;

    assert!(matches!(
        events.as_slice(),
        [
            ProviderEvent::ResponseStarted {
                response: Some(response_id),
            },
            ProviderEvent::FunctionCallRequested(call),
        ]
            if response_id == "resp_tool"
                && call.response_id == "resp_tool"
                && call.call_id == "call_shell"
                && call.name == "shell"
    ));
}

#[tokio::test]
async fn parses_summary_stream() {
    let events = capture_events(vec![
        r#"data: {"type":"response.created","response":{"id":"resp_reasoning"}}"#,
        r#"data: {"type":"response.reasoning_summary_text.delta","delta":"looking "}"#,
        r#"data: {"type":"response.reasoning_summary_text.delta","delta":"closely"}"#,
        r#"data: {"type":"response.reasoning_summary_text.done","text":"looking closely"}"#,
    ])
    .await;

    assert!(matches!(
        events.as_slice(),
        [
            ProviderEvent::ResponseStarted { .. },
            ProviderEvent::ReasoningSummaryDelta(first),
            ProviderEvent::ReasoningSummaryDelta(second),
            ProviderEvent::ReasoningSummaryDone(done),
        ] if first == "looking " && second == "closely" && done == "looking closely"
    ));
}

#[tokio::test]
async fn parses_summary_item_done() {
    let events = capture_events(vec![
        r#"data: {"type":"response.output_item.done","item":{"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"First. "},{"type":"summary_text","text":"Second."}]}}"#,
    ])
    .await;

    assert!(matches!(
        events.as_slice(),
        [ProviderEvent::ReasoningSummaryDone(summary)] if summary == "First. Second."
    ));
}

#[tokio::test]
async fn emits_stream_trace_events() {
    let events = capture_all_events(vec![
        r#"data: {"type":"response.created","response":{"id":"resp_trace"}}"#,
        r#"data: {"type":"response.output_text.delta","delta":"ok"}"#,
    ])
    .await;

    assert!(events.iter().any(|event| {
        matches!(
            event,
            ProviderEvent::StreamTrace(ProviderStreamTrace::Chunk { .. })
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            ProviderEvent::StreamTrace(ProviderStreamTrace::RawEvent {
                raw_type,
                mapped_events,
            }) if raw_type == "response.created"
                && mapped_events == &vec!["response_started".to_string()]
        )
    }));
}

#[tokio::test]
async fn poisoned_replay_regenerates() {
    let item = serde_json::json!({
        "type": "function_call",
        "call_id": "call_1",
        "name": "shell",
        "arguments": "{}",
        "id": "item_deadbeef"
    });
    let function_call = capture_replay(Some(item)).await;
    match function_call.get("id").and_then(Value::as_str) {
        None => {}
        Some(id) => assert!(id.starts_with("fc"), "must not forward invalid id: {id}"),
    }
    assert_eq!(function_call["call_id"], "call_1");
    assert_eq!(function_call["name"], "shell");
}

#[tokio::test]
async fn valid_replay_survives() {
    let item = serde_json::json!({
        "type": "function_call",
        "call_id": "call_1",
        "name": "shell",
        "arguments": "{}",
        "id": "fc_ok"
    });
    let function_call = capture_replay(Some(item)).await;
    assert_eq!(function_call["id"], "fc_ok");
}

#[tokio::test]
async fn absent_replay_synthesizes() {
    let function_call = capture_replay(None).await;
    assert!(function_call.get("id").is_none());
    assert_eq!(function_call["call_id"], "call_1");
}

#[path = "openai/support.rs"]
mod support;
use support::*;
