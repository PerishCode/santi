use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::mpsc,
    thread,
};

use futures_util::StreamExt;
use santi_provider::{
    ChatCompletionsProvider, ChatCompletionsProviderConfig, ProviderClient, ProviderEvent,
    ProviderFunctionTool, ProviderItem, ProviderRequest, ProviderTool,
};
use serde_json::Value;

pub(crate) async fn capture_body(mut config: ChatCompletionsProviderConfig) -> Value {
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

pub(crate) async fn capture_with_items(items: Vec<ProviderItem>) -> Value {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let config = ChatCompletionsProviderConfig {
        provider: "deepseek".to_string(),
        api_key: "test-key".to_string(),
        model: "deepseek-v4-pro".to_string(),
        base_url: format!("http://{}", listener.local_addr().expect("local address")),
        thinking: None,
        reasoning_effort: None,
        max_tokens: None,
        input_budget_bytes: None,
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

pub(crate) fn function_call_item(call_id: &str, command: &str) -> ProviderItem {
    ProviderItem::FunctionCall {
        call_id: call_id.to_string(),
        name: "shell".to_string(),
        arguments_raw: format!(r#"{{"command":"{command}"}}"#),
        item: None,
        item_id: None,
    }
}

pub(crate) async fn capture_events(lines: Vec<&'static str>) -> Vec<ProviderEvent> {
    capture_all_events(lines)
        .await
        .into_iter()
        .filter(|event| !matches!(event, ProviderEvent::StreamTrace(_)))
        .collect()
}

pub(crate) async fn capture_all_events(lines: Vec<&'static str>) -> Vec<ProviderEvent> {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let config = ChatCompletionsProviderConfig {
        provider: "deepseek".to_string(),
        api_key: "test-key".to_string(),
        model: "deepseek-v4-pro".to_string(),
        base_url: format!("http://{}", listener.local_addr().expect("local address")),
        thinking: None,
        reasoning_effort: None,
        max_tokens: None,
        input_budget_bytes: None,
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

pub(crate) fn base_request(model: String, extra_items: Vec<ProviderItem>) -> ProviderRequest {
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

pub(crate) fn response_server(
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

pub(crate) async fn next_business_event(
    stream: &mut santi_provider::ProviderStream,
) -> Option<ProviderEvent> {
    while let Some(event) = stream.next().await {
        let event = event.expect("provider event");
        if !matches!(event, ProviderEvent::StreamTrace(_)) {
            return Some(event);
        }
    }
    None
}

pub(crate) async fn assert_completed(stream: &mut santi_provider::ProviderStream) {
    while let Some(event) = next_business_event(stream).await {
        if matches!(event, ProviderEvent::Completed { .. }) {
            return;
        }
    }
    panic!("expected completed provider event");
}

pub(crate) fn read_body(stream: &mut impl Read) -> Vec<u8> {
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
