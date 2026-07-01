//! `santi` is the single entry point for the runtime.
//!
//! It has two faces, split by subcommand:
//!
//! - `santi service ...` runs the runtime server in-process (delegated to the
//!   `santi-api` crate, which owns the HTTP boundary and links `santi-core`).
//! - every other command is a transport-only HTTP client: it reaches the
//!   runtime exclusively over HTTP against a running server, never calling
//!   `santi-core` in-process. HTTP stays the only way into the runtime.

use std::collections::HashSet;
use std::io::Write as _;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use futures_util::{Stream, StreamExt};

/// Grace window after the in-flight turn set empties, before declaring the
/// soul_session idle. A completed turn re-checks for newer requests and may
/// start a follow-on turn (coalescing); its `turn_started` lands just after the
/// `turn_completed`, so we wait briefly to catch it rather than exit early.
const WATCH_IDLE_GRACE: Duration = Duration::from_millis(1500);

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:43307";

#[derive(Parser)]
#[command(
    name = "santi",
    version,
    about = "santi runtime: server (`service`) and HTTP client"
)]
struct Cli {
    /// Base URL of a running santi server. Falls back to SANTI_API_URL, then
    /// the local default. Only used by the HTTP client commands.
    #[arg(long, global = true, env = "SANTI_API_URL", default_value = DEFAULT_BASE_URL)]
    base_url: String,

    /// Bearer token sent on client requests when the server requires one.
    /// Falls back to SANTI_API_KEY. Only used by the HTTP client commands.
    #[arg(long, global = true, env = "SANTI_API_KEY")]
    api_key: Option<String>,

    /// Default session id used when a session subcommand omits an explicit id.
    /// Falls back to SANTI_SESSION_ID. Empty/absent → an id must be passed.
    #[arg(long, global = true, env = "SANTI_SESSION_ID")]
    session: Option<String>,

    /// Default soul addressed by `session send`. Falls back to SANTI_SOUL_ID.
    /// Empty/absent → the runtime's default soul (the pre-multi-soul path).
    #[arg(long, global = true, env = "SANTI_SOUL_ID")]
    soul: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the runtime server in-process (`serve`, `export-openapi`).
    Service {
        /// Arguments forwarded to the server (e.g. `serve`, `export-openapi`,
        /// `--config`, `--provider`).
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// GET /api/v1/health
    Health,
    /// Session resources under /api/v1/sessions
    #[command(subcommand)]
    Session(SessionCommand),
}

#[derive(Subcommand)]
enum SessionCommand {
    /// POST /api/v1/sessions
    Create,
    /// GET /api/v1/sessions
    List,
    /// GET /api/v1/sessions/{id} (id falls back to --session/SANTI_SESSION_ID)
    Get { id: Option<String> },
    /// GET /api/v1/sessions/{id}/messages (id falls back to --session)
    Messages { id: Option<String> },
    /// GET /api/v1/sessions/{id}/runtime (id falls back to --session)
    Runtime { id: Option<String> },
    /// POST /api/v1/sessions/{id}/send.
    ///
    /// Positional forms: `send <id> <text>` or `send <text>` (id then falls
    /// back to --session/SANTI_SESSION_ID). Soul comes from --soul/SANTI_SOUL_ID.
    Send {
        /// Either `<id> <text>` or just `<text>`.
        #[arg(num_args = 1..=2, required = true)]
        args: Vec<String>,
        /// After sending, follow the stream until the soul_session goes idle,
        /// then exit. Robust to coalescing and silent (speechless) completions.
        #[arg(long)]
        watch: bool,
    },
    /// GET /api/v1/sessions/{id}/events — follows the SSE stream (id falls back
    /// to --session). Runs until interrupted; use `send --watch` to stop on idle.
    Events { id: Option<String> },
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv_override().ok();
    let cli = Cli::parse();
    match cli.command {
        Command::Service { args } => run_service(args).await,
        other => {
            let defaults = ClientDefaults {
                session: cli.session,
                soul: cli.soul,
            };
            run_client(&cli.base_url, cli.api_key.as_deref(), &defaults, other).await
        }
    }
}

/// Client-side defaults resolved from global flags / env. They never reach the
/// runtime as concepts: `session` only fills an omitted path id, and `soul` is
/// forwarded on `send` (empty → the runtime keeps its default-soul path).
struct ClientDefaults {
    session: Option<String>,
    soul: Option<String>,
}

impl ClientDefaults {
    /// Resolve a session id: an explicit positional wins, else the default.
    /// Both empty is a usage error — same "you must name a session" path as before.
    fn resolve_session(&self, explicit: Option<String>) -> Result<String> {
        explicit
            .or_else(|| self.session.clone())
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!("no session id: pass one or set --session / SANTI_SESSION_ID")
            })
    }

    /// The soul to address, or None to let the runtime use its default soul.
    fn soul(&self) -> Option<&str> {
        self.soul
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }
}

/// Run the runtime server in-process via `santi-api`.
async fn run_service(args: Vec<String>) -> Result<()> {
    let argv = std::iter::once("santi".to_string()).chain(args);
    let config = santi_api::config::ConfigService::from_args(argv)
        .map_err(|error| anyhow::anyhow!(error))?;
    match config.command() {
        santi_api::config::AppCommand::Serve => santi_api::serve(config)
            .await
            .map_err(|error| anyhow::anyhow!(error)),
        santi_api::config::AppCommand::ExportOpenApi => {
            let document =
                santi_api::export_openapi_json().map_err(|error| anyhow::anyhow!(error))?;
            println!("{document}");
            Ok(())
        }
    }
}

/// Transport-only HTTP client against a running server.
async fn run_client(
    base_url: &str,
    api_key: Option<&str>,
    defaults: &ClientDefaults,
    command: Command,
) -> Result<()> {
    let client = build_client(api_key)?;
    let base = base_url.trim_end_matches('/').to_string();
    match command {
        Command::Service { .. } => unreachable!("service is handled before the client path"),
        Command::Health => get(&client, &format!("{base}/api/v1/health")).await,
        Command::Session(SessionCommand::Create) => {
            post(&client, &format!("{base}/api/v1/sessions"), None).await
        }
        Command::Session(SessionCommand::List) => {
            get(&client, &format!("{base}/api/v1/sessions")).await
        }
        Command::Session(SessionCommand::Get { id }) => {
            let id = defaults.resolve_session(id)?;
            get(&client, &format!("{base}/api/v1/sessions/{id}")).await
        }
        Command::Session(SessionCommand::Messages { id }) => {
            let id = defaults.resolve_session(id)?;
            get(&client, &format!("{base}/api/v1/sessions/{id}/messages")).await
        }
        Command::Session(SessionCommand::Runtime { id }) => {
            let id = defaults.resolve_session(id)?;
            get(&client, &format!("{base}/api/v1/sessions/{id}/runtime")).await
        }
        Command::Session(SessionCommand::Send { args, watch }) => {
            let (id, text) = split_send_args(args, defaults)?;
            let mut content = serde_json::json!({
                "content": [{ "type": "text", "text": text }]
            });
            if let Some(soul) = defaults.soul() {
                content["soul_id"] = serde_json::Value::from(soul);
            }
            send(&client, &base, &id, content, watch).await
        }
        Command::Session(SessionCommand::Events { id }) => {
            let id = defaults.resolve_session(id)?;
            follow(&client, &format!("{base}/api/v1/sessions/{id}/events")).await
        }
    }
}

/// Split `send` positionals into `(session_id, text)`. Two args = explicit
/// `<id> <text>`; one arg = `<text>` with the id from --session/SANTI_SESSION_ID.
fn split_send_args(mut args: Vec<String>, defaults: &ClientDefaults) -> Result<(String, String)> {
    match args.len() {
        2 => {
            let text = args.pop().expect("len == 2");
            let id = args.pop().expect("len == 2");
            Ok((defaults.resolve_session(Some(id))?, text))
        }
        1 => {
            let text = args.pop().expect("len == 1");
            Ok((defaults.resolve_session(None)?, text))
        }
        _ => anyhow::bail!("send takes `<id> <text>` or `<text>`"),
    }
}

/// Build an HTTP client that attaches `Authorization: Bearer <key>` to every
/// request when an api key is configured.
fn build_client(api_key: Option<&str>) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder();
    if let Some(key) = api_key {
        let mut headers = reqwest::header::HeaderMap::new();
        let mut value = reqwest::header::HeaderValue::from_str(&format!("Bearer {key}"))
            .context("invalid api key")?;
        value.set_sensitive(true);
        headers.insert(reqwest::header::AUTHORIZATION, value);
        builder = builder.default_headers(headers);
    }
    builder.build().context("build http client")
}

async fn get(client: &reqwest::Client, url: &str) -> Result<()> {
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    print_json(response).await
}

async fn post(client: &reqwest::Client, url: &str, body: Option<serde_json::Value>) -> Result<()> {
    let mut request = client.post(url);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .with_context(|| format!("POST {url}"))?;
    print_json(response).await
}

async fn print_json(response: reqwest::Response) -> Result<()> {
    let status = response.status();
    let text = response.text().await.context("read response body")?;
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(value) => println!("{}", serde_json::to_string_pretty(&value)?),
        Err(_) => println!("{text}"),
    }
    if !status.is_success() {
        anyhow::bail!("request failed with status {status}");
    }
    Ok(())
}

/// Stream a server-sent-event endpoint, writing raw bytes through as they
/// arrive. The client does not parse or model the events; it is a pipe.
async fn follow(client: &reqwest::Client, url: &str) -> Result<()> {
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("request failed with status {status}");
    }
    let mut stream = response.bytes_stream();
    let mut stdout = std::io::stdout();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("read event stream")?;
        stdout.write_all(&chunk).context("write event chunk")?;
        stdout.flush().ok();
    }
    Ok(())
}

/// POST a send, then optionally `--watch` the stream until the soul_session is
/// idle again. Without `--watch` this is the prior fire-and-return behavior.
async fn send(
    client: &reqwest::Client,
    base: &str,
    session_id: &str,
    body: serde_json::Value,
    watch: bool,
) -> Result<()> {
    let url = format!("{base}/api/v1/sessions/{session_id}/send");
    let response = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("POST {url}"))?;
    let status = response.status();
    let text = response.text().await.context("read response body")?;
    let accepted = serde_json::from_str::<serde_json::Value>(&text).ok();
    if !watch {
        match &accepted {
            Some(value) => println!("{}", serde_json::to_string_pretty(value)?),
            None => println!("{text}"),
        }
    }
    if !status.is_success() {
        if watch {
            println!("{text}");
        }
        anyhow::bail!("request failed with status {status}");
    }
    if !watch {
        return Ok(());
    }
    // Seed the in-flight set with the turn this send landed on (a fresh turn, or
    // the running one it coalesced into), so a follow-on that handles our message
    // is still awaited even if its `turn_started` arrives after the seed's end.
    let seed_turn = accepted
        .as_ref()
        .and_then(|value| value.get("turn"))
        .and_then(|turn| turn.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    watch_until_idle(client, base, session_id, seed_turn).await
}

/// Follow the session's SSE stream, tracking which turns are in flight, and
/// return once none remain (the soul_session has caught up). Each event is
/// relayed to stdout as one compact JSON line; the client models only the
/// turn-lifecycle events it needs to decide "idle", nothing more.
async fn watch_until_idle(
    client: &reqwest::Client,
    base: &str,
    session_id: &str,
    seed_turn: Option<String>,
) -> Result<()> {
    let url = format!("{base}/api/v1/sessions/{session_id}/events");
    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("request failed with status {status}");
    }
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut inflight: HashSet<String> = HashSet::new();
    let mut seeded = false;
    if let Some(turn) = seed_turn {
        inflight.insert(turn);
        seeded = true;
    }
    let mut stdout = std::io::stdout();
    loop {
        // Once nothing is in flight, allow only a short grace for a coalesced
        // follow-on turn before declaring idle. Before seeding (no known turn),
        // wait without a deadline so we don't exit before the turn appears.
        let frame = if seeded && inflight.is_empty() {
            match tokio::time::timeout(WATCH_IDLE_GRACE, next_sse_frame(&mut stream, &mut buffer))
                .await
            {
                Ok(frame) => frame?,
                Err(_) => break,
            }
        } else {
            next_sse_frame(&mut stream, &mut buffer).await?
        };
        let Some((event, data)) = frame else {
            break; // stream closed
        };
        if event != "stream_open" {
            writeln!(stdout, "{data}").ok();
            stdout.flush().ok();
        }
        match event.as_str() {
            "turn_started" => {
                if let Some(id) = json_field(&data, &["payload", "turn", "id"]) {
                    inflight.insert(id);
                    seeded = true;
                }
            }
            "turn_completed" | "turn_failed" => {
                if let Some(id) = json_field(&data, &["payload", "turn_id"]) {
                    inflight.remove(&id);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Pull the next complete SSE frame, returning its `(event, data)` lines.
/// Comment-only frames (keep-alives) and frames without an event are skipped.
/// Returns `Ok(None)` when the stream ends.
async fn next_sse_frame<B: AsRef<[u8]>>(
    stream: &mut (impl Stream<Item = reqwest::Result<B>> + Unpin),
    buffer: &mut String,
) -> Result<Option<(String, String)>> {
    loop {
        while let Some(boundary) = buffer.find("\n\n") {
            let frame: String = buffer.drain(..boundary + 2).collect();
            if let Some(parsed) = parse_sse_frame(&frame) {
                return Ok(Some(parsed));
            }
        }
        match stream.next().await {
            Some(chunk) => {
                let chunk = chunk.context("read event stream")?;
                buffer.push_str(&String::from_utf8_lossy(chunk.as_ref()));
            }
            None => return Ok(None),
        }
    }
}

/// Parse one SSE frame into `(event, data)`. Returns None if it has no event
/// line (e.g. a `:` keep-alive comment).
fn parse_sse_frame(frame: &str) -> Option<(String, String)> {
    let mut event = None;
    let mut data = String::new();
    for line in frame.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            event = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    event.map(|event| (event, data))
}

/// Read a nested string field from a compact JSON document by key path.
fn json_field(data: &str, path: &[&str]) -> Option<String> {
    let mut value = serde_json::from_str::<serde_json::Value>(data).ok()?;
    for key in path {
        value = value.get(key)?.clone();
    }
    value.as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults(session: Option<&str>, soul: Option<&str>) -> ClientDefaults {
        ClientDefaults {
            session: session.map(str::to_string),
            soul: soul.map(str::to_string),
        }
    }

    #[test]
    fn resolve_session_prefers_explicit_then_default() {
        let d = defaults(Some("sess_default"), None);
        assert_eq!(d.resolve_session(Some("sess_x".into())).unwrap(), "sess_x");
        assert_eq!(d.resolve_session(None).unwrap(), "sess_default");
    }

    #[test]
    fn resolve_session_errors_when_both_empty() {
        assert!(defaults(None, None).resolve_session(None).is_err());
        // A blank default is treated as absent, not a valid id.
        assert!(defaults(Some("  "), None).resolve_session(None).is_err());
    }

    #[test]
    fn soul_blank_is_none() {
        assert_eq!(defaults(None, Some("soul_x")).soul(), Some("soul_x"));
        assert_eq!(defaults(None, Some("   ")).soul(), None);
        assert_eq!(defaults(None, None).soul(), None);
    }

    #[test]
    fn split_send_args_two_then_one() {
        let d = defaults(Some("sess_default"), None);
        let (id, text) = split_send_args(vec!["sess_x".into(), "hi".into()], &d).unwrap();
        assert_eq!((id.as_str(), text.as_str()), ("sess_x", "hi"));

        let (id, text) = split_send_args(vec!["hello".into()], &d).unwrap();
        assert_eq!((id.as_str(), text.as_str()), ("sess_default", "hello"));

        // One arg with no default session is a usage error, not a silent send.
        assert!(split_send_args(vec!["hello".into()], &defaults(None, None)).is_err());
    }

    #[test]
    fn parse_sse_frame_extracts_event_and_data() {
        let frame = "id: e1\nevent: turn_completed\ndata: {\"payload\":{\"turn_id\":\"t1\"}}\n";
        let (event, data) = parse_sse_frame(frame).expect("frame");
        assert_eq!(event, "turn_completed");
        assert_eq!(data, "{\"payload\":{\"turn_id\":\"t1\"}}");
        // A keep-alive comment frame has no event line.
        assert!(parse_sse_frame(": keep-alive\n").is_none());
    }

    #[test]
    fn json_field_reads_nested_path() {
        let data = "{\"payload\":{\"turn\":{\"id\":\"t9\"}}}";
        assert_eq!(
            json_field(data, &["payload", "turn", "id"]).as_deref(),
            Some("t9")
        );
        assert_eq!(json_field(data, &["payload", "missing"]), None);
    }

    #[tokio::test]
    async fn next_sse_frame_yields_frames_across_chunk_boundaries() {
        use futures_util::stream;
        // A frame split across two chunks, plus a trailing comment-only frame.
        let chunks: Vec<reqwest::Result<Vec<u8>>> = vec![
            Ok(b"event: turn_started\ndata: {\"payl".to_vec()),
            Ok(b"oad\":{\"turn\":{\"id\":\"t1\"}}}\n\n: ka\n\n".to_vec()),
        ];
        let mut s = stream::iter(chunks);
        let mut buf = String::new();
        let (event, data) = next_sse_frame(&mut s, &mut buf).await.unwrap().unwrap();
        assert_eq!(event, "turn_started");
        assert_eq!(
            json_field(&data, &["payload", "turn", "id"]).as_deref(),
            Some("t1")
        );
        // Only the keep-alive remains → no further parsable frame.
        assert!(next_sse_frame(&mut s, &mut buf).await.unwrap().is_none());
    }
}
