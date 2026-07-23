use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::mpsc,
    thread,
};

use futures_util::StreamExt;
use santi_provider::{
    OpenAIProvider, OpenAIProviderConfig, ProviderClient, ProviderEvent, ProviderFunctionTool,
    ProviderItem, ProviderRequest, ProviderTool,
};
use serde_json::Value;

pub(crate) async fn capture_body(mut config: OpenAIProviderConfig) -> Value {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    config.base_url = format!("http://{}", listener.local_addr().expect("local address"));
    let (tx, rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        let body = read_body(&mut stream);
        tx.send(body).expect("send request body");
        let event = r#"data: {"type":"response.completed","response":{"id":"resp_test"}}"#;
        let response_body = format!("{event}\n\n");
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream
            .write_all(response.as_bytes())
            .expect("write response");
    });

    let provider = OpenAIProvider::new(config);
    let mut stream = provider
        .stream_response(ProviderRequest {
            model: provider.metadata().model,
            instructions: Some("system guidance".to_string()),
            input: vec![ProviderItem::Message {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
            tools: Some(vec![ProviderTool::Function(ProviderFunctionTool {
                name: "shell".to_string(),
                description: "run shell".to_string(),
                parameters: serde_json::json!({ "type": "object" }),
            })]),
            previous_response_id: None,
        })
        .await
        .expect("stream response");
    assert!(matches!(
        next_business_event(&mut stream).await,
        Some(ProviderEvent::Completed { .. })
    ));

    let body = rx.recv().expect("receive request body");
    server.join().expect("server thread");
    serde_json::from_slice(&body).expect("json request")
}

pub(crate) async fn capture_body_without_tools(mut config: OpenAIProviderConfig) -> Value {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    config.base_url = format!("http://{}", listener.local_addr().expect("local address"));
    let (tx, rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        let body = read_body(&mut stream);
        tx.send(body).expect("send request body");
        let event = r#"data: {"type":"response.completed","response":{"id":"resp_test"}}"#;
        let response_body = format!("{event}\n\n");
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream
            .write_all(response.as_bytes())
            .expect("write response");
    });

    let provider = OpenAIProvider::new(config);
    let mut stream = provider
        .stream_response(ProviderRequest {
            model: provider.metadata().model,
            instructions: Some("system guidance".to_string()),
            input: vec![ProviderItem::Message {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
            tools: None,
            previous_response_id: None,
        })
        .await
        .expect("stream response");
    assert!(matches!(
        next_business_event(&mut stream).await,
        Some(ProviderEvent::Completed { .. })
    ));

    let body = rx.recv().expect("receive request body");
    server.join().expect("server thread");
    serde_json::from_slice(&body).expect("json request")
}

pub(crate) async fn capture_replay(item: Option<Value>) -> Value {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let config = OpenAIProviderConfig {
        api_key: "test-key".to_string(),
        model: "gpt-5.5".to_string(),
        base_url: format!("http://{}", listener.local_addr().expect("local address")),
        reasoning_effort: None,
        reasoning_summary: None,
        max_output_tokens: None,
        bytes: None,
    };
    let (tx, rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        tx.send(read_body(&mut stream)).expect("send request body");
        let event = r#"data: {"type":"response.completed","response":{"id":"resp_test"}}"#;
        let response_body = format!("{event}\n\n");
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream
            .write_all(response.as_bytes())
            .expect("write response");
    });

    let provider = OpenAIProvider::new(config);
    let mut stream = provider
        .stream_response(ProviderRequest {
            model: provider.metadata().model,
            instructions: None,
            input: vec![ProviderItem::FunctionCall {
                call_id: "call_1".to_string(),
                name: "shell".to_string(),
                arguments_raw: "{}".to_string(),
                item,
                mark: None,
            }],
            tools: None,
            previous_response_id: None,
        })
        .await
        .expect("stream response");
    assert!(matches!(
        next_business_event(&mut stream).await,
        Some(ProviderEvent::Completed { .. })
    ));

    let body: Value = serde_json::from_slice(&rx.recv().expect("request body")).expect("json");
    server.join().expect("server thread");
    body["input"]
        .as_array()
        .expect("input array")
        .iter()
        .find(|item| item["type"] == "function_call")
        .expect("function call")
        .clone()
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
    let config = OpenAIProviderConfig {
        api_key: "test-key".to_string(),
        model: "gpt-5.5".to_string(),
        base_url: format!("http://{}", listener.local_addr().expect("local address")),
        reasoning_effort: None,
        reasoning_summary: None,
        max_output_tokens: None,
        bytes: None,
    };
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        let _ = read_body(&mut stream);
        let response_body = format!("{}\n\n", lines.join("\n\n"));
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream
            .write_all(response.as_bytes())
            .expect("write response");
    });

    let provider = OpenAIProvider::new(config);
    let mut stream = provider
        .stream_response(ProviderRequest {
            model: provider.metadata().model,
            instructions: None,
            input: vec![ProviderItem::Message {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
            tools: None,
            previous_response_id: None,
        })
        .await
        .expect("stream response");
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event.expect("provider event"));
    }
    server.join().expect("server thread");
    events
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
