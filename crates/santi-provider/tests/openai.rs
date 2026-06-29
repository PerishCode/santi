use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::mpsc,
    thread,
};

use futures_util::StreamExt;
use santi_provider::{
    OpenAIProvider, OpenAIProviderConfig, ProviderClient, ProviderEvent, ProviderFunctionTool,
    ProviderMessage, ProviderRequest, ProviderStreamTrace, ProviderTool,
};
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
                provider_response_id: Some(response_id),
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

async fn capture_body(mut config: OpenAIProviderConfig) -> Value {
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
            input: vec![ProviderMessage {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
            tools: Some(vec![ProviderTool::Function(ProviderFunctionTool {
                name: "shell".to_string(),
                description: "run shell".to_string(),
                parameters: serde_json::json!({ "type": "object" }),
            })]),
            previous_response_id: None,
            function_call_outputs: None,
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

async fn capture_body_without_tools(mut config: OpenAIProviderConfig) -> Value {
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
            input: vec![ProviderMessage {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
            tools: None,
            previous_response_id: None,
            function_call_outputs: None,
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

async fn capture_events(lines: Vec<&'static str>) -> Vec<ProviderEvent> {
    capture_all_events(lines)
        .await
        .into_iter()
        .filter(|event| !matches!(event, ProviderEvent::StreamTrace(_)))
        .collect()
}

async fn capture_all_events(lines: Vec<&'static str>) -> Vec<ProviderEvent> {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let config = OpenAIProviderConfig {
        api_key: "test-key".to_string(),
        model: "gpt-5.5".to_string(),
        base_url: format!("http://{}", listener.local_addr().expect("local address")),
        reasoning_effort: None,
        reasoning_summary: None,
        max_output_tokens: None,
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
            input: vec![ProviderMessage {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
            tools: None,
            previous_response_id: None,
            function_call_outputs: None,
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

async fn next_business_event(stream: &mut santi_provider::ProviderStream) -> Option<ProviderEvent> {
    while let Some(event) = stream.next().await {
        let event = event.expect("provider event");
        if !matches!(event, ProviderEvent::StreamTrace(_)) {
            return Some(event);
        }
    }
    None
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
