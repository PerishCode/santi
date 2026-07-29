use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use santi::cli::WatchFormat;
use santi::client::{Request, Target, send};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

#[derive(Clone)]
enum FakeEventsResponse {
    CompletesSeedTurn,
    ClosesImmediately,
    Status500,
    AcceptedWarning,
}

struct CountingHttpServer {
    base_url: String,
    post_send_count: Arc<AtomicUsize>,
    get_events_count: Arc<AtomicUsize>,
}

async fn spawn_server(events_response: FakeEventsResponse) -> CountingHttpServer {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fake server");
    let addr = listener.local_addr().expect("fake server addr");
    let post_send_count = Arc::new(AtomicUsize::new(0));
    let get_events_count = Arc::new(AtomicUsize::new(0));
    let server_post_count = post_send_count.clone();
    let server_get_count = get_events_count.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let post_count = server_post_count.clone();
            let get_count = server_get_count.clone();
            let events_response = events_response.clone();
            tokio::spawn(async move {
                handle_request(stream, post_count, get_count, events_response).await;
            });
        }
    });
    CountingHttpServer {
        base_url: format!("http://{addr}"),
        post_send_count,
        get_events_count,
    }
}

async fn handle_request(
    mut stream: TcpStream,
    post_send_count: Arc<AtomicUsize>,
    get_events_count: Arc<AtomicUsize>,
    events_response: FakeEventsResponse,
) {
    let Some((method, path)) = read_request_line(&mut stream).await else {
        return;
    };
    if method == "POST" && path == "/api/v1/strands/ss_cli/send" {
        post_send_count.fetch_add(1, Ordering::SeqCst);
        let body = if matches!(&events_response, FakeEventsResponse::AcceptedWarning) {
            r#"{"receipt":{"warning":{"code":"runtime.strand.drive_failed","context":{"recovery":{"command":"santi strand drive ss_cli"}}}}}"#
        } else {
            r#"{"turn":{"id":"turn_seed"}}"#
        };
        write_response(&mut stream, "200 OK", "application/json", body).await;
    } else if method == "GET" && path == "/api/v1/strands/ss_cli/events" {
        get_events_count.fetch_add(1, Ordering::SeqCst);
        match events_response {
            FakeEventsResponse::CompletesSeedTurn => {
                write_response(
                    &mut stream,
                    "200 OK",
                    "text/event-stream",
                    "event: turn\ndata: {\"payload\":{\"type\":\"turn\",\"beat\":\"completed\",\"turn\":\"turn_seed\"}}\n\n",
                )
                .await;
            }
            FakeEventsResponse::ClosesImmediately => {
                write_response(&mut stream, "200 OK", "text/event-stream", "").await;
            }
            FakeEventsResponse::Status500 => {
                write_response(
                    &mut stream,
                    "500 Internal Server Error",
                    "text/plain",
                    "boom",
                )
                .await;
            }
            FakeEventsResponse::AcceptedWarning => unreachable!("warning stops before watch"),
        }
    } else {
        write_response(&mut stream, "404 Not Found", "text/plain", "not found").await;
    }
}

async fn read_request_line(stream: &mut TcpStream) -> Option<(String, String)> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let head = String::from_utf8_lossy(&buffer);
    let mut parts = head.lines().next()?.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    Some((method, path))
}

async fn write_response(stream: &mut TcpStream, status: &str, content_type: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .expect("write fake response");
}

#[tokio::test]
async fn completes() {
    let server = spawn_server(FakeEventsResponse::CompletesSeedTurn).await;
    let client = reqwest::Client::new();

    send(Request {
        target: Target::new(&client, &server.base_url, "ss_cli", WatchFormat::Raw),
        body: serde_json::json!({"content":[{"type":"text","text":"hello"}]}),
        watch: true,
    })
    .await
    .expect("send --watch succeeds");

    assert_eq!(server.post_send_count.load(Ordering::SeqCst), 1);
    assert_eq!(server.get_events_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn closes() {
    let server = spawn_server(FakeEventsResponse::ClosesImmediately).await;
    let client = reqwest::Client::new();

    send(Request {
        target: Target::new(&client, &server.base_url, "ss_cli", WatchFormat::Raw),
        body: serde_json::json!({"content":[{"type":"text","text":"hello"}]}),
        watch: true,
    })
    .await
    .expect("closed watch stream is not retried as a send");

    assert_eq!(server.post_send_count.load(Ordering::SeqCst), 1);
    assert_eq!(server.get_events_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn errors() {
    let server = spawn_server(FakeEventsResponse::Status500).await;
    let client = reqwest::Client::new();

    let error = send(Request {
        target: Target::new(&client, &server.base_url, "ss_cli", WatchFormat::Raw),
        body: serde_json::json!({"content":[{"type":"text","text":"hello"}]}),
        watch: true,
    })
    .await
    .expect_err("watch failure should surface to caller");

    assert!(error.to_string().contains("request failed with status"));
    assert_eq!(server.post_send_count.load(Ordering::SeqCst), 1);
    assert_eq!(server.get_events_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn warns() {
    let server = spawn_server(FakeEventsResponse::AcceptedWarning).await;
    let client = reqwest::Client::new();

    let error = send(Request {
        target: Target::new(&client, &server.base_url, "ss_cli", WatchFormat::Raw),
        body: serde_json::json!({"content":[{"type":"text","text":"hello"}]}),
        watch: true,
    })
    .await
    .expect_err("accepted driver warning must require explicit recovery");

    assert!(error.to_string().contains("do not resend"));
    assert!(error.to_string().contains("santi strand drive ss_cli"));
    assert_eq!(server.post_send_count.load(Ordering::SeqCst), 1);
    assert_eq!(server.get_events_count.load(Ordering::SeqCst), 0);
}
