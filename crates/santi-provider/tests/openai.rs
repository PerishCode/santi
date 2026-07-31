use santi_provider::openai::Config;
use santi_provider::{Event, Trace};
use serde_json::Value;

#[tokio::test]
async fn sent() {
    let body = posted(Config {
        key: "test-key".to_string(),
        model: "gpt-5.5".to_string(),
        url: String::new(),
        effort: Some("medium".to_string()),
        summary: Some("auto".to_string()),
        ceiling: Some(4096),
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
async fn omitted() {
    let body = posted(Config {
        key: "test-key".to_string(),
        model: "gpt-4.1".to_string(),
        url: String::new(),
        effort: None,
        summary: None,
        ceiling: None,
        bytes: None,
    })
    .await;

    assert!(body.get("reasoning").is_none());
    assert!(body.get("max_output_tokens").is_none());
    assert_eq!(body["store"], false);
}

#[tokio::test]
async fn unstored() {
    let body = bare(Config {
        key: "test-key".to_string(),
        model: "gpt-4.1".to_string(),
        url: String::new(),
        effort: None,
        summary: None,
        ceiling: None,
        bytes: None,
    })
    .await;

    assert_eq!(body["store"], false);
}

#[tokio::test]
async fn identified() {
    let events = events(vec![
        r#"data: {"type":"response.created","response":{"id":"resp_tool"}}"#,
        r#"data: {"type":"response.output_item.done","item":{"type":"function_call","id":"item_shell","call_id":"call_shell","name":"shell","arguments":"{\"cmd\":\"pwd\"}"}}"#,
    ])
    .await;

    let [
        Event::Started {
            response: Some(response),
        },
        Event::Called(call),
    ] = events.as_slice()
    else {
        panic!("unexpected identified event sequence");
    };
    assert_eq!(response, "resp_tool");
    assert_eq!(call.response, "resp_tool");
    assert_eq!(call.call, "call_shell");
    assert_eq!(call.name, "shell");
}

#[tokio::test]
async fn summaries() {
    let events = events(vec![
        r#"data: {"type":"response.created","response":{"id":"resp_reasoning"}}"#,
        r#"data: {"type":"response.reasoning_summary_text.delta","delta":"looking "}"#,
        r#"data: {"type":"response.reasoning_summary_text.delta","delta":"closely"}"#,
        r#"data: {"type":"response.reasoning_summary_text.done","text":"looking closely"}"#,
    ])
    .await;

    assert!(matches!(
        events.as_slice(),
        [
            Event::Started { .. },
            Event::Thinking(first),
            Event::Thinking(second),
            Event::Thought(done),
        ] if first == "looking " && second == "closely" && done == "looking closely"
    ));
}

#[tokio::test]
async fn summarized() {
    let events = events(vec![
        r#"data: {"type":"response.output_item.done","item":{"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"First. "},{"type":"summary_text","text":"Second."}]}}"#,
    ])
    .await;

    assert!(matches!(
        events.as_slice(),
        [Event::Thought(summary)] if summary == "First. Second."
    ));
}

#[tokio::test]
async fn traced() {
    let events = captured(vec![
        r#"data: {"type":"response.created","response":{"id":"resp_trace"}}"#,
        r#"data: {"type":"response.output_text.delta","delta":"ok"}"#,
    ])
    .await;

    assert!(
        events
            .iter()
            .any(|event| { matches!(event, Event::Traced(Trace::Chunk { .. })) })
    );
    assert!(events.iter().any(|event| {
        matches!(
            event,
            Event::Traced(Trace::Raw {
                kind,
                mapped,
            }) if kind == "response.created"
                && mapped == &vec!["response_started".to_string()]
        )
    }));
}

#[tokio::test]
async fn regenerated() {
    let item = serde_json::json!({
        "type": "function_call",
        "call_id": "call_1",
        "name": "shell",
        "arguments": "{}",
        "id": "item_deadbeef"
    });
    let called = replayed(Some(item)).await;
    match called.get("id").and_then(Value::as_str) {
        None => {}
        Some(id) => assert!(id.starts_with("fc"), "must not forward invalid id: {id}"),
    }
    assert_eq!(called["call_id"], "call_1");
    assert_eq!(called["name"], "shell");
}

#[tokio::test]
async fn survived() {
    let item = serde_json::json!({
        "type": "function_call",
        "call_id": "call_1",
        "name": "shell",
        "arguments": "{}",
        "id": "fc_ok"
    });
    let called = replayed(Some(item)).await;
    assert_eq!(called["id"], "fc_ok");
}

#[tokio::test]
async fn synthesized() {
    let called = replayed(None).await;
    assert!(called.get("id").is_none());
    assert_eq!(called["call_id"], "call_1");
}

#[path = "openai/support.rs"]
mod support;
use support::*;
