use santi_provider::chat::completions::Config;
use santi_provider::{Event, Item, Trace};

#[tokio::test]
async fn mapped() {
    let body = posted(Config {
        provider: "deepseek".to_string(),
        key: "test-key".to_string(),
        model: "deepseek-v4-pro".to_string(),
        url: String::new(),
        thinking: Some("disabled".to_string()),
        effort: Some("high".to_string()),
        ceiling: Some(512),
        bytes: None,
    })
    .await;

    assert_eq!(body["model"], "deepseek-v4-pro");
    assert_eq!(body["stream"], true);
    assert_eq!(body["thinking"]["type"], "disabled");
    assert_eq!(body["reasoning_effort"], "high");
    assert_eq!(body["max_tokens"], 512);
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(body["messages"][1]["role"], "user");
    assert_eq!(body["tools"][0]["type"], "function");
    assert_eq!(body["tools"][0]["function"]["name"], "shell");
}

#[tokio::test]
async fn flattened() {
    let body = itemized(vec![
        Item::Call {
            call: "call_shell".to_string(),
            name: "shell".to_string(),
            raw: "{\"command\":\"pwd\"}".to_string(),
            item: None,
            mark: None,
        },
        Item::Output {
            call: "call_shell".to_string(),
            output: "/tmp".to_string(),
        },
    ])
    .await;

    assert_eq!(body["messages"][2]["role"], "assistant");
    assert!(body["messages"][2]["content"].is_null());
    assert_eq!(body["messages"][2]["tool_calls"][0]["id"], "call_shell");
    assert_eq!(
        body["messages"][2]["tool_calls"][0]["function"]["arguments"],
        "{\"command\":\"pwd\"}"
    );
    assert_eq!(body["messages"][3]["role"], "tool");
    assert_eq!(body["messages"][3]["tool_call_id"], "call_shell");
    assert_eq!(body["messages"][3]["content"], "/tmp");
}

#[tokio::test]
async fn interleaved() {
    let body = itemized(vec![
        fixture("call_one", "pwd"),
        Item::Output {
            call: "call_one".to_string(),
            output: "one".to_string(),
        },
        fixture("call_two", "ls"),
        Item::Output {
            call: "call_two".to_string(),
            output: "two".to_string(),
        },
    ])
    .await;

    assert_eq!(body["messages"][2]["role"], "assistant");
    assert_eq!(body["messages"][2]["tool_calls"][0]["id"], "call_one");
    assert_eq!(body["messages"][3]["role"], "tool");
    assert_eq!(body["messages"][3]["tool_call_id"], "call_one");

    assert_eq!(body["messages"][4]["role"], "assistant");
    assert_eq!(body["messages"][4]["tool_calls"][0]["id"], "call_two");
    assert_eq!(body["messages"][5]["role"], "tool");
    assert_eq!(body["messages"][5]["tool_call_id"], "call_two");
}

#[tokio::test]
async fn reasoned() {
    let events = events(vec![
        r#"data: {"id":"chatcmpl_1","choices":[{"delta":{"role":"assistant"},"finish_reason":null}]}"#,
        r#"data: {"id":"chatcmpl_1","choices":[{"delta":{"reasoning_content":"thinking"},"finish_reason":null}]}"#,
        r#"data: {"id":"chatcmpl_1","choices":[{"delta":{"content":"ok"},"finish_reason":null}]}"#,
        r#"data: {"id":"chatcmpl_1","choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        r#"data: [DONE]"#,
    ])
    .await;

    let [
        Event::Started {
            response: Some(response),
        },
        Event::Thinking(reasoning),
        Event::Text(text),
        Event::Completed {
            response: Some(completed_id),
        },
    ] = events.as_slice()
    else {
        panic!("unexpected reasoned event sequence");
    };
    assert_eq!(reasoning, "thinking");
    assert_eq!(text, "ok");
    assert_eq!(response, "chatcmpl_1");
    assert_eq!(completed_id, "chatcmpl_1");
}

#[tokio::test]
async fn streamed() {
    let events = events(vec![
        r#"data: {"id":"chatcmpl_tool","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_shell","type":"function","function":{"name":"shell","arguments":"{\"command\""}}]},"finish_reason":null}]}"#,
        r#"data: {"id":"chatcmpl_tool","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":":\"pwd\"}"}}]},"finish_reason":null}]}"#,
        r#"data: {"id":"chatcmpl_tool","choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
    ])
    .await;

    let [Event::Started { .. }, Event::Called(call)] = events.as_slice() else {
        panic!("unexpected streamed event sequence");
    };
    assert_eq!(call.response, "chatcmpl_tool");
    assert_eq!(call.call, "call_shell");
    assert_eq!(call.name, "shell");
    assert_eq!(call.arguments["command"], "pwd");
}

#[tokio::test]
async fn named() {
    let events = events(vec![
        r#"data: {"id":"chatcmpl_tool","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_shell","type":"function","function":{"name":"shell","arguments":""}}]},"finish_reason":null}]}"#,
        r#"data: {"id":"chatcmpl_tool","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"","arguments":"{\"command\""}}]},"finish_reason":null}]}"#,
        r#"data: {"id":"chatcmpl_tool","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"","arguments":":\"pwd\"}"}}]},"finish_reason":null}]}"#,
        r#"data: {"id":"chatcmpl_tool","choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
    ])
    .await;

    assert!(matches!(
        events.as_slice(),
        [
            Event::Started { .. },
            Event::Called(call),
        ] if call.name == "shell"
                && call.arguments["command"] == "pwd"
    ));
}

#[tokio::test]
async fn traced() {
    let events = captured(vec![
        r#"data: {"id":"chatcmpl_1","choices":[{"delta":{"content":"ok"},"finish_reason":null}]}"#,
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
            }) if kind == "chat.completion.chunk"
                && mapped == &vec![
                    "response_started".to_string(),
                    "text_delta".to_string(),
                ]
        )
    }));
}

#[path = "chat/support.rs"]
mod support;
use support::*;
