use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::mpsc,
    thread,
};

use futures_util::StreamExt;
use santi_provider::{
    ChatCompletionsProvider, ChatCompletionsProviderConfig, ProviderClient, ProviderEvent,
    ProviderFunctionTool, ProviderItem, ProviderRequest, ProviderStreamTrace, ProviderTool,
};
use serde_json::Value;

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

    // Reasoning is dropped (DC5); the function call flattens to an assistant
    // message with null content + tool_calls, followed by the tool result.
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

async fn capture_body(mut config: ChatCompletionsProviderConfig) -> Value {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    config.base_url = format!("http://{}", listener.local_addr().expect("local address"));
    let (tx, rx) = mpsc::channel();
    let server = response_server(
        listener,
        tx,
        vec![r#"data: {"id":"chatcmpl_test","choices":[{"delta":{},"finish_reason":"stop"}]}"#],
    );

    let provider = ChatCompletionsProvider::new(config);
    let mut stream = provider
        .stream_response(base_request(provider.metadata().model, vec![]))
        .await
        .expect("stream response");
    assert_completed(&mut stream).await;

    let body = rx.recv().expect("receive request body");
    server.join().expect("server thread");
    serde_json::from_slice(&body).expect("json request")
}

async fn capture_with_items(items: Vec<ProviderItem>) -> Value {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let config = ChatCompletionsProviderConfig {
        provider: "deepseek".to_string(),
        api_key: "test-key".to_string(),
        model: "deepseek-v4-pro".to_string(),
        base_url: format!("http://{}", listener.local_addr().expect("local address")),
        thinking: None,
        reasoning_effort: None,
        max_tokens: None,
    };
    let (tx, rx) = mpsc::channel();
    let server = response_server(
        listener,
        tx,
        vec![r#"data: {"id":"chatcmpl_test","choices":[{"delta":{},"finish_reason":"stop"}]}"#],
    );

    let provider = ChatCompletionsProvider::new(config);
    let mut stream = provider
        .stream_response(base_request(provider.metadata().model, items))
        .await
        .expect("stream response");
    assert_completed(&mut stream).await;

    let body = rx.recv().expect("receive request body");
    server.join().expect("server thread");
    serde_json::from_slice(&body).expect("json request")
}

fn function_call_item(call_id: &str, command: &str) -> ProviderItem {
    ProviderItem::FunctionCall {
        call_id: call_id.to_string(),
        name: "shell".to_string(),
        arguments_raw: format!(r#"{{"command":"{command}"}}"#),
        item: None,
        item_id: None,
    }
}

async fn capture_events(lines: Vec<&'static str>) -> Vec<ProviderEvent> {
    capture_all_events(lines)
        .await
        .into_iter()
        .filter(|event| !matches!(event, ProviderEvent::StreamTrace(_)))
        .collect()
}

async fn capture_all_events(lines: Vec<&'static str>) -> Vec<ProviderEvent> {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let config = ChatCompletionsProviderConfig {
        provider: "deepseek".to_string(),
        api_key: "test-key".to_string(),
        model: "deepseek-v4-pro".to_string(),
        base_url: format!("http://{}", listener.local_addr().expect("local address")),
        thinking: None,
        reasoning_effort: None,
        max_tokens: None,
    };
    let (tx, rx) = mpsc::channel();
    let server = response_server(listener, tx, lines);

    let provider = ChatCompletionsProvider::new(config);
    let mut stream = provider
        .stream_response(base_request(provider.metadata().model, vec![]))
        .await
        .expect("stream response");
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event.expect("provider event"));
    }
    let _ = rx.recv().expect("receive request body");
    server.join().expect("server thread");
    events
}

fn base_request(model: String, extra_items: Vec<ProviderItem>) -> ProviderRequest {
    let mut input = vec![ProviderItem::Message {
        role: "user".to_string(),
        content: "hello".to_string(),
    }];
    input.extend(extra_items);
    ProviderRequest {
        model,
        instructions: Some("system guidance".to_string()),
        input,
        tools: Some(vec![ProviderTool::Function(ProviderFunctionTool {
            name: "shell".to_string(),
            description: "run shell".to_string(),
            parameters: serde_json::json!({ "type": "object" }),
        })]),
        previous_response_id: None,
    }
}

fn response_server(
    listener: TcpListener,
    tx: mpsc::Sender<Vec<u8>>,
    lines: Vec<&'static str>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        let body = read_body(&mut stream);
        tx.send(body).expect("send request body");
        let response_body = format!("{}\n\n", lines.join("\n\n"));
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream
            .write_all(response.as_bytes())
            .expect("write response");
    })
}

async fn next_business_event(stream: &mut santi_provider::ProviderStream) -> Option<ProviderEvent> {
    while let Some(event) = stream.next().await {
        let event = event.expect("provider event");
        if !matches!(event, ProviderEvent::StreamTrace(_)) {
            return Some(event);
        }
    }
    None
}

async fn assert_completed(stream: &mut santi_provider::ProviderStream) {
    while let Some(event) = next_business_event(stream).await {
        if matches!(event, ProviderEvent::Completed { .. }) {
            return;
        }
    }
    panic!("expected completed provider event");
}

fn read_body(stream: &mut impl Read) -> Vec<u8> {
    let mut request = Vec::new();
    let mut buffer = [0; 1024];
    loop {
        let read = stream.read(&mut buffer).expect("read request");
        assert!(read > 0, "connection closed before headers");
        request.extend_from_slice(&buffer[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }

    let header_end = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("header end")
        + 4;
    let headers = String::from_utf8_lossy(&request[..header_end]);
    let length = headers
        .lines()
        .find_map(|line| {
            line.strip_prefix("content-length:")
                .or_else(|| line.strip_prefix("Content-Length:"))
        })
        .expect("content length")
        .trim()
        .parse::<usize>()
        .expect("content length value");

    while request.len() - header_end < length {
        let read = stream.read(&mut buffer).expect("read body");
        assert!(read > 0, "connection closed before body");
        request.extend_from_slice(&buffer[..read]);
    }
    request[header_end..header_end + length].to_vec()
}
