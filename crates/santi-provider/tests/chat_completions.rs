use santi_provider::{
    ChatCompletionsProviderConfig, ProviderEvent, ProviderItem, ProviderStreamTrace,
};

#[tokio::test]
async fn maps_chat_body() {
    let body = capture_body(ChatCompletionsProviderConfig {
        provider: "deepseek".to_string(),
        api_key: "test-key".to_string(),
        model: "deepseek-v4-pro".to_string(),
        base_url: String::new(),
        thinking: Some("disabled".to_string()),
        reasoning_effort: Some("high".to_string()),
        max_tokens: Some(512),
        input_budget_bytes: None,
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
async fn flattens_tool_items() {
    let body = capture_with_items(vec![
        ProviderItem::FunctionCall {
            call_id: "call_shell".to_string(),
            name: "shell".to_string(),
            arguments_raw: "{\"command\":\"pwd\"}".to_string(),
            item: None,
            item_id: None,
        },
        ProviderItem::FunctionCallOutput {
            call_id: "call_shell".to_string(),
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
async fn flattens_interleaved_rounds() {
    let body = capture_with_items(vec![
        function_call_item("call_one", "pwd"),
        ProviderItem::FunctionCallOutput {
            call_id: "call_one".to_string(),
            output: "one".to_string(),
        },
        function_call_item("call_two", "ls"),
        ProviderItem::FunctionCallOutput {
            call_id: "call_two".to_string(),
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
async fn parses_reasoning_text() {
    let events = capture_events(vec![
        r#"data: {"id":"chatcmpl_1","choices":[{"delta":{"role":"assistant"},"finish_reason":null}]}"#,
        r#"data: {"id":"chatcmpl_1","choices":[{"delta":{"reasoning_content":"thinking"},"finish_reason":null}]}"#,
        r#"data: {"id":"chatcmpl_1","choices":[{"delta":{"content":"ok"},"finish_reason":null}]}"#,
        r#"data: {"id":"chatcmpl_1","choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        r#"data: [DONE]"#,
    ])
    .await;

    assert!(matches!(
        events.as_slice(),
        [
            ProviderEvent::ResponseStarted {
                provider_response_id: Some(response_id),
            },
            ProviderEvent::ReasoningSummaryDelta(reasoning),
            ProviderEvent::TextDelta(text),
            ProviderEvent::Completed {
                provider_response_id: Some(completed_id),
            },
        ] if reasoning == "thinking"
            && text == "ok"
            && response_id == "chatcmpl_1"
            && completed_id == "chatcmpl_1"
    ));
}

#[tokio::test]
async fn parses_streamed_tool_call() {
    let events = capture_events(vec![
        r#"data: {"id":"chatcmpl_tool","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_shell","type":"function","function":{"name":"shell","arguments":"{\"command\""}}]},"finish_reason":null}]}"#,
        r#"data: {"id":"chatcmpl_tool","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":":\"pwd\"}"}}]},"finish_reason":null}]}"#,
        r#"data: {"id":"chatcmpl_tool","choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
    ])
    .await;

    assert!(matches!(
        events.as_slice(),
        [
            ProviderEvent::ResponseStarted { .. },
            ProviderEvent::FunctionCallRequested(call),
        ] if call.response_id == "chatcmpl_tool"
                && call.call_id == "call_shell"
                && call.name == "shell"
                && call.arguments["command"] == "pwd"
    ));
}

#[tokio::test]
async fn keeps_tool_name() {
    let events = capture_events(vec![
        r#"data: {"id":"chatcmpl_tool","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_shell","type":"function","function":{"name":"shell","arguments":""}}]},"finish_reason":null}]}"#,
        r#"data: {"id":"chatcmpl_tool","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"","arguments":"{\"command\""}}]},"finish_reason":null}]}"#,
        r#"data: {"id":"chatcmpl_tool","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"","arguments":":\"pwd\"}"}}]},"finish_reason":null}]}"#,
        r#"data: {"id":"chatcmpl_tool","choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
    ])
    .await;

    assert!(matches!(
        events.as_slice(),
        [
            ProviderEvent::ResponseStarted { .. },
            ProviderEvent::FunctionCallRequested(call),
        ] if call.name == "shell"
                && call.arguments["command"] == "pwd"
    ));
}

#[tokio::test]
async fn emits_stream_trace_events() {
    let events = capture_all_events(vec![
        r#"data: {"id":"chatcmpl_1","choices":[{"delta":{"content":"ok"},"finish_reason":null}]}"#,
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
            }) if raw_type == "chat.completion.chunk"
                && mapped_events == &vec![
                    "response_started".to_string(),
                    "text_delta".to_string(),
                ]
        )
    }));
}

#[path = "chat_completions/support.rs"]
mod support;
use support::*;
