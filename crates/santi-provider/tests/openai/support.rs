use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::mpsc,
    thread,
};

use futures_util::StreamExt;
use santi_provider::openai::{Config, OpenAI};
use santi_provider::{Event, Function, Item, Provider, Request, Tool};
use serde_json::Value;

pub(crate) async fn posted(mut config: Config) -> Value {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    config.url = format!("http://{}", listener.local_addr().expect("local address"));
    let (tx, rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        let held = body(&mut stream);
        tx.send(held).expect("send request body");
        let event = r#"data: {"type":"response.completed","response":{"id":"resp_test"}}"#;
        let body = format!("{event}\n\n");
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .expect("write response");
    });

    let provider = OpenAI::new(config);
    let mut stream = provider
        .stream(Request {
            model: provider.metadata().model,
            instructions: Some("system guidance".to_string()),
            input: vec![Item::Message {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
            tools: Some(vec![Tool::Function(Function {
                name: "shell".to_string(),
                description: "run shell".to_string(),
                parameters: serde_json::json!({ "type": "object" }),
            })]),
            previous: None,
        })
        .await
        .expect("stream response");
    assert!(matches!(
        next(&mut stream).await,
        Some(Event::Completed { .. })
    ));

    let body = rx.recv().expect("receive request body");
    server.join().expect("server thread");
    serde_json::from_slice(&body).expect("json request")
}

pub(crate) async fn bare(mut config: Config) -> Value {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    config.url = format!("http://{}", listener.local_addr().expect("local address"));
    let (tx, rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        let held = body(&mut stream);
        tx.send(held).expect("send request body");
        let event = r#"data: {"type":"response.completed","response":{"id":"resp_test"}}"#;
        let body = format!("{event}\n\n");
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .expect("write response");
    });

    let provider = OpenAI::new(config);
    let mut stream = provider
        .stream(Request {
            model: provider.metadata().model,
            instructions: Some("system guidance".to_string()),
            input: vec![Item::Message {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
            tools: None,
            previous: None,
        })
        .await
        .expect("stream response");
    assert!(matches!(
        next(&mut stream).await,
        Some(Event::Completed { .. })
    ));

    let body = rx.recv().expect("receive request body");
    server.join().expect("server thread");
    serde_json::from_slice(&body).expect("json request")
}

pub(crate) async fn replayed(item: Option<Value>) -> Value {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let config = Config {
        key: "test-key".to_string(),
        model: "gpt-5.5".to_string(),
        url: format!("http://{}", listener.local_addr().expect("local address")),
        effort: None,
        summary: None,
        ceiling: None,
        bytes: None,
    };
    let (tx, rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        tx.send(body(&mut stream)).expect("send request body");
        let event = r#"data: {"type":"response.completed","response":{"id":"resp_test"}}"#;
        let body = format!("{event}\n\n");
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .expect("write response");
    });

    let provider = OpenAI::new(config);
    let mut stream = provider
        .stream(Request {
            model: provider.metadata().model,
            instructions: None,
            input: vec![Item::Call {
                call: "call_1".to_string(),
                name: "shell".to_string(),
                raw: "{}".to_string(),
                item,
                mark: None,
            }],
            tools: None,
            previous: None,
        })
        .await
        .expect("stream response");
    assert!(matches!(
        next(&mut stream).await,
        Some(Event::Completed { .. })
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

pub(crate) async fn events(lines: Vec<&'static str>) -> Vec<Event> {
    captured(lines)
        .await
        .into_iter()
        .filter(|event| !matches!(event, Event::Traced(_)))
        .collect()
}

pub(crate) async fn captured(lines: Vec<&'static str>) -> Vec<Event> {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let config = Config {
        key: "test-key".to_string(),
        model: "gpt-5.5".to_string(),
        url: format!("http://{}", listener.local_addr().expect("local address")),
        effort: None,
        summary: None,
        ceiling: None,
        bytes: None,
    };
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        let _ = body(&mut stream);
        let body = format!("{}\n\n", lines.join("\n\n"));
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .expect("write response");
    });

    let provider = OpenAI::new(config);
    let mut stream = provider
        .stream(Request {
            model: provider.metadata().model,
            instructions: None,
            input: vec![Item::Message {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
            tools: None,
            previous: None,
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

pub(crate) async fn next(stream: &mut santi_provider::Streaming) -> Option<Event> {
    while let Some(event) = stream.next().await {
        let event = event.expect("provider event");
        if !matches!(event, Event::Traced(_)) {
            return Some(event);
        }
    }
    None
}

pub(crate) fn body(stream: &mut impl Read) -> Vec<u8> {
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

    let split = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("header end")
        + 4;
    let headers = String::from_utf8_lossy(&request[..split]);
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

    while request.len() - split < length {
        let read = stream.read(&mut buffer).expect("read body");
        assert!(read > 0, "connection closed before body");
        request.extend_from_slice(&buffer[..read]);
    }
    request[split..split + length].to_vec()
}
