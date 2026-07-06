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
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use futures_util::{Stream, StreamExt};

/// Grace window after the in-flight turn set empties, before declaring the
/// strand idle. A completed turn re-checks for newer requests and may
/// start a follow-on turn (coalescing); its `turn_started` lands just after the
/// `turn_completed`, so we wait briefly to catch it rather than exit early.
const WATCH_IDLE_GRACE: Duration = Duration::from_millis(1500);

/// Per-request timeout for the immediate (non-streaming) commands. Generous for
/// a snapshot read, yet bounded so a request that never gets answered fails fast
/// instead of hanging forever — notably a client command run from inside the
/// soul's own turn, which connects back to the turn-holding server (self-call);
/// the timeout turns that wedge into a fast error the turn can move past. The
/// streaming path (`follow`) is deliberately exempt: SSE is a long-lived body.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

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

    /// Static bearer token sent on client requests. Falls back to SANTI_API_KEY.
    /// Transitional: santi itself no longer gates on this; prefer the edge-auth
    /// (authentik client_credentials) flags below to reach santi behind forward-auth.
    #[arg(long, global = true, env = "SANTI_API_KEY")]
    api_key: Option<String>,

    /// Edge auth via authentik client_credentials. When token-url, client-id,
    /// username AND password are all set, the client exchanges them for a
    /// short-lived JWT (cached locally, ~1h) and sends THAT as the bearer instead
    /// of --api-key — the way to reach santi behind authentik forward-auth.
    #[arg(long, global = true, env = "SANTI_AUTH_TOKEN_URL")]
    auth_token_url: Option<String>,
    #[arg(long, global = true, env = "SANTI_AUTH_CLIENT_ID")]
    auth_client_id: Option<String>,
    #[arg(long, global = true, env = "SANTI_AUTH_USERNAME")]
    auth_username: Option<String>,
    #[arg(long, global = true, env = "SANTI_AUTH_PASSWORD")]
    auth_password: Option<String>,

    /// Default strand id used when a strand subcommand omits an explicit id.
    /// Falls back to SANTI_STRAND_ID. Empty/absent → an id must be passed.
    #[arg(long, global = true, env = "SANTI_STRAND_ID")]
    strand: Option<String>,

    /// Default soul addressed by `strand send`. Falls back to SANTI_SOUL_ID.
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
    /// Offline pre-check of the on-disk store + default soul memory (read-only).
    /// A local ops command (NOT an HTTP client): exits non-zero when unhealthy,
    /// so the upgrade flow can gate on it. See PHASE-07.
    Doctor,
    /// Offline store-level ops (act directly on the DB, no running service).
    #[command(subcommand)]
    Inbox(InboxCommand),
    /// Self-upgrade (PHASE-07). Without `--run`: launch the detached upgrade unit
    /// and return fast with a signal. With `--run`: the orchestration itself (what
    /// the shipped `santi-upgrade.service` oneshot unit invokes). Local ops.
    Upgrade {
        /// The `.deb` to install (path or, later, a downloaded artifact).
        deb: Option<String>,
        /// Run the orchestration in-process instead of launching the unit.
        #[arg(long)]
        run: bool,
    },
    /// GET /api/v1/health
    Health,
    /// Strand resources under /api/v1/strands
    #[command(subcommand)]
    Strand(StrandCommand),
    /// Compact a strand's own timeline, or query a compact's detail.
    #[command(subcommand)]
    Compact(CompactCommand),
    /// The plain IM integrated into santi — converse with a soul as a persistent
    /// participant (send/poll over HTTP), or the soul's offline reply egress.
    #[command(subcommand)]
    Im(ImCommand),
}

#[derive(Subcommand)]
enum ImCommand {
    /// Send a message to a soul via the integrated IM, as a persistent
    /// participant, and (with --reply) wait for the soul's reply. Target soul
    /// from --soul/SANTI_SOUL_ID; participant from --as/SANTI_IM_PARTICIPANT.
    Send {
        /// The message text.
        text: String,
        /// Your persistent participant id (your IM identity + reply address).
        #[arg(long = "as", env = "SANTI_IM_PARTICIPANT", default_value = "operator")]
        participant: String,
        /// After sending, poll your inbox until the soul replies (or timeout).
        #[arg(long)]
        reply: bool,
        /// Max seconds to wait for a reply with --reply (turns run minutes).
        #[arg(long, default_value_t = 300)]
        reply_timeout: u64,
    },
    /// Poll a participant's IM inbox once — entries past --since (0 = all).
    Poll {
        /// The participant id whose inbox to read.
        #[arg(long = "as", env = "SANTI_IM_PARTICIPANT", default_value = "operator")]
        participant: String,
        #[arg(long, default_value_t = 0)]
        since: i64,
    },
    /// The soul's egress: reply into the current IM conversation's participant
    /// inbox. OFFLINE (a direct store write, no HTTP — a mid-turn reply never
    /// re-enters the turn-holding server). Conversation from --strand/SANTI_STRAND_ID.
    Reply {
        /// The reply text.
        text: String,
    },
}

#[derive(Subcommand)]
enum CompactCommand {
    /// POST /api/v1/strands/{id}/compact — collapse [from,to] into a summary.
    /// Strand from --strand/SANTI_STRAND_ID; soul from --soul/SANTI_SOUL_ID.
    Exec {
        /// First message of the range (a fixed user/assistant message id).
        #[arg(long)]
        from: String,
        /// Last message of the range (a fixed user/assistant message id).
        #[arg(long)]
        to: String,
        /// The summary text. Mutually exclusive with --summary-file.
        #[arg(
            long,
            conflicts_with = "summary_file",
            required_unless_present = "summary_file"
        )]
        summary: Option<String>,
        /// Read the summary from a file (or `-` for stdin) instead of --summary.
        #[arg(long)]
        summary_file: Option<String>,
    },
    /// GET /api/v1/compacts/{id} — expand a compact's covered range (paginated).
    Query {
        #[arg(long)]
        compact_id: String,
        #[arg(long)]
        keyword: Option<String>,
        #[arg(long, default_value_t = 0)]
        page_index: i64,
        #[arg(long, default_value_t = 50)]
        page_size: i64,
    },
}

#[derive(Subcommand)]
enum InboxCommand {
    /// Enqueue one `santi_system` record into a strand's durable inbox WITHOUT a
    /// running service (a direct MQ producer). The strand comes from
    /// --strand/SANTI_STRAND_ID and must already exist. Used by the self-upgrade
    /// flow to seed the "come look" record before starting the final version.
    Seed {
        /// The message text (the "come look" occurrence).
        text: String,
    },
}

#[derive(Subcommand)]
enum StrandCommand {
    /// POST /api/v1/strands
    Create,
    /// GET /api/v1/strands
    List,
    /// GET /api/v1/strands/{id} (id falls back to --strand/SANTI_STRAND_ID)
    Get { id: Option<String> },
    /// GET /api/v1/strands/{id}/messages (id falls back to --strand)
    Messages { id: Option<String> },
    /// GET /api/v1/strands/{id}/runtime (id falls back to --strand)
    Runtime { id: Option<String> },
    /// POST /api/v1/strands/{id}/send.
    ///
    /// Positional forms: `send <id> <text>` or `send <text>` (id then falls
    /// back to --strand/SANTI_STRAND_ID). Soul comes from --soul/SANTI_SOUL_ID.
    Send {
        /// Either `<id> <text>` or just `<text>`.
        #[arg(num_args = 1..=2, required = true)]
        args: Vec<String>,
        /// After sending, follow the stream until the strand goes idle,
        /// then exit. Robust to coalescing and silent (speechless) completions.
        #[arg(long)]
        watch: bool,
    },
    /// GET /api/v1/strands/{id}/events — follows the SSE stream (id falls back
    /// to --strand). Runs until interrupted; use `send --watch` to stop on idle.
    Events { id: Option<String> },
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv_override().ok();
    let cli = Cli::parse();
    match cli.command {
        Command::Service { args } => run_service(args).await,
        Command::Doctor => run_doctor(),
        Command::Inbox(inbox) => run_inbox(inbox, cli.strand),
        Command::Upgrade { deb, run } => run_upgrade(deb, run),
        // The soul's IM reply is an offline store write (like `inbox seed`) — no
        // HTTP, so a mid-turn reply never re-enters the turn-holding server.
        Command::Im(ImCommand::Reply { text }) => run_im_reply(text, cli.strand),
        other => {
            let defaults = ClientDefaults {
                strand: cli.strand,
                soul: cli.soul,
            };
            let bearer = resolve_edge_bearer(
                cli.auth_token_url.as_deref(),
                cli.auth_client_id.as_deref(),
                cli.auth_username.as_deref(),
                cli.auth_password.as_deref(),
                cli.api_key.as_deref(),
            )
            .await?;
            run_client(&cli.base_url, bearer.as_deref(), &defaults, other).await
        }
    }
}

/// Client-side defaults resolved from global flags / env. They never reach the
/// runtime as concepts: `strand` only fills an omitted path id, and `soul` is
/// forwarded on `send` (empty → the runtime keeps its default-soul path).
struct ClientDefaults {
    strand: Option<String>,
    soul: Option<String>,
}

impl ClientDefaults {
    /// Resolve a strand id: an explicit positional wins, else the default.
    /// Both empty is a usage error — same "you must name a strand" path as before.
    fn resolve_strand(&self, explicit: Option<String>) -> Result<String> {
        explicit
            .or_else(|| self.strand.clone())
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!("no strand id: pass one or set --strand / SANTI_STRAND_ID")
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

/// Offline pre-check (local ops, no HTTP). Prints the report as JSON to stdout
/// and exits non-zero when unhealthy, so a caller (the upgrade flow) can gate.
fn run_doctor() -> Result<()> {
    let report = santi_api::ops::doctor().map_err(|error| anyhow::anyhow!(error))?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    if !report.ok {
        anyhow::bail!("doctor: unhealthy (see report above)");
    }
    Ok(())
}

/// Offline inbox producer (local ops, no HTTP). Resolves the strand from
/// --strand/SANTI_STRAND_ID and seeds a durable record; exits non-zero if the
/// inbox gate rejects it, so the upgrade flow notices a badly-behind strand.
fn run_inbox(command: InboxCommand, default_strand: Option<String>) -> Result<()> {
    match command {
        InboxCommand::Seed { text } => {
            let strand_id = default_strand
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty())
                .ok_or_else(|| anyhow::anyhow!("no strand id: set --strand / SANTI_STRAND_ID"))?;
            let report = santi_api::ops::inbox_seed(&strand_id, &text)
                .map_err(|error| anyhow::anyhow!(error))?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if !report.accepted {
                anyhow::bail!("inbox seed rejected: {}", report.reason.unwrap_or_default());
            }
            Ok(())
        }
    }
}

/// The soul's IM reply egress (local ops, no HTTP). Resolves the current IM
/// conversation from --strand/SANTI_STRAND_ID (ambient in the soul's shell) and
/// delivers the reply into that conversation's participant inbox — a direct store
/// write, so a mid-turn reply never re-enters the turn-holding server.
fn run_im_reply(text: String, default_strand: Option<String>) -> Result<()> {
    let strand_id = default_strand
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("no strand id: set --strand / SANTI_STRAND_ID (the IM conversation)")
        })?;
    let report =
        santi_api::ops::im_reply(&strand_id, &text).map_err(|error| anyhow::anyhow!(error))?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

/// Self-upgrade (local ops, no HTTP). `--run` executes the orchestration (what
/// the oneshot unit calls); otherwise it launches that unit detached and returns
/// the fast signal (监听 / 最长超时 Xmin / 日志位置).
fn run_upgrade(deb: Option<String>, run: bool) -> Result<()> {
    if run {
        let report = santi_api::upgrade::run(deb).map_err(|error| anyhow::anyhow!(error))?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        Ok(())
    } else {
        let deb = deb.ok_or_else(|| anyhow::anyhow!("usage: santi upgrade <deb> [--run]"))?;
        let started = santi_api::upgrade::launch(&deb).map_err(|error| anyhow::anyhow!(error))?;
        println!("{}", serde_json::to_string_pretty(&started)?);
        Ok(())
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
    bearer: Option<&str>,
    defaults: &ClientDefaults,
    command: Command,
) -> Result<()> {
    let client = build_client(bearer)?;
    let base = base_url.trim_end_matches('/').to_string();
    match command {
        Command::Service { .. } => unreachable!("service is handled before the client path"),
        Command::Doctor => unreachable!("doctor is handled before the client path"),
        Command::Inbox(_) => unreachable!("inbox is handled before the client path"),
        Command::Upgrade { .. } => unreachable!("upgrade is handled before the client path"),
        Command::Health => get(&client, &format!("{base}/api/v1/health")).await,
        Command::Strand(StrandCommand::Create) => {
            post(&client, &format!("{base}/api/v1/strands"), None).await
        }
        Command::Strand(StrandCommand::List) => {
            get(&client, &format!("{base}/api/v1/strands")).await
        }
        Command::Strand(StrandCommand::Get { id }) => {
            let id = defaults.resolve_strand(id)?;
            get(&client, &format!("{base}/api/v1/strands/{id}")).await
        }
        Command::Strand(StrandCommand::Messages { id }) => {
            let id = defaults.resolve_strand(id)?;
            get(&client, &format!("{base}/api/v1/strands/{id}/messages")).await
        }
        Command::Strand(StrandCommand::Runtime { id }) => {
            let id = defaults.resolve_strand(id)?;
            get(&client, &format!("{base}/api/v1/strands/{id}/runtime")).await
        }
        Command::Strand(StrandCommand::Send { args, watch }) => {
            let (id, text) = split_send_args(args, defaults)?;
            let mut content = serde_json::json!({
                "content": [{ "type": "text", "text": text }]
            });
            if let Some(soul) = defaults.soul() {
                content["soul_id"] = serde_json::Value::from(soul);
            }
            send(&client, &base, &id, content, watch).await
        }
        Command::Strand(StrandCommand::Events { id }) => {
            let id = defaults.resolve_strand(id)?;
            follow(&client, &format!("{base}/api/v1/strands/{id}/events")).await
        }
        Command::Im(ImCommand::Send {
            text,
            participant,
            reply,
            reply_timeout,
        }) => {
            let soul = defaults
                .soul()
                .ok_or_else(|| anyhow::anyhow!("no target soul: set --soul / SANTI_SOUL_ID"))?;
            let body = serde_json::json!({
                "soul_id": soul,
                "participant_id": participant,
                "content": text,
            });
            im_send(&client, &base, body, &participant, reply, reply_timeout).await
        }
        Command::Im(ImCommand::Poll { participant, since }) => {
            get(
                &client,
                &format!("{base}/api/v1/im/inbox/{participant}?since={since}"),
            )
            .await
        }
        Command::Im(ImCommand::Reply { .. }) => {
            unreachable!("im reply is handled before the client path")
        }
        Command::Compact(CompactCommand::Exec {
            from,
            to,
            summary,
            summary_file,
        }) => {
            let strand = defaults.resolve_strand(None)?;
            let summary = match summary_file {
                Some(path) => read_summary_file(&path)?,
                None => summary.expect("clap requires summary or summary_file"),
            };
            let mut body = serde_json::json!({
                "from_message_id": from,
                "to_message_id": to,
                "summary": summary,
            });
            if let Some(soul) = defaults.soul() {
                body["soul_id"] = serde_json::Value::from(soul);
            }
            post(
                &client,
                &format!("{base}/api/v1/strands/{strand}/compact"),
                Some(body),
            )
            .await
        }
        Command::Compact(CompactCommand::Query {
            compact_id,
            keyword,
            page_index,
            page_size,
        }) => {
            let mut url = format!(
                "{base}/api/v1/compacts/{compact_id}?page_index={page_index}&page_size={page_size}"
            );
            if let Some(keyword) = keyword.filter(|k| !k.is_empty()) {
                url.push_str(&format!("&keyword={}", urlencoding_encode(&keyword)));
            }
            get(&client, &url).await
        }
    }
}

/// Read a compact summary from a file, or stdin when the path is `-`.
fn read_summary_file(path: &str) -> Result<String> {
    if path == "-" {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)
            .context("read summary from stdin")?;
        Ok(buf)
    } else {
        std::fs::read_to_string(path).with_context(|| format!("read summary file {path}"))
    }
}

/// Minimal percent-encoding for a query-string value (the client has no URL dep).
fn urlencoding_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Split `send` positionals into `(strand_id, text)`. Two args = explicit
/// `<id> <text>`; one arg = `<text>` with the id from --strand/SANTI_STRAND_ID.
fn split_send_args(mut args: Vec<String>, defaults: &ClientDefaults) -> Result<(String, String)> {
    match args.len() {
        2 => {
            let text = args.pop().expect("len == 2");
            let id = args.pop().expect("len == 2");
            Ok((defaults.resolve_strand(Some(id))?, text))
        }
        1 => {
            let text = args.pop().expect("len == 1");
            Ok((defaults.resolve_strand(None)?, text))
        }
        _ => anyhow::bail!("send takes `<id> <text>` or `<text>`"),
    }
}

/// Build an HTTP client that attaches `Authorization: Bearer <token>` to every
/// request when a bearer is configured.
fn build_client(bearer: Option<&str>) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder();
    if let Some(token) = bearer {
        let mut headers = reqwest::header::HeaderMap::new();
        let mut value = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
            .context("invalid bearer token")?;
        value.set_sensitive(true);
        headers.insert(reqwest::header::AUTHORIZATION, value);
        builder = builder.default_headers(headers);
    }
    builder.build().context("build http client")
}

/// Resolve the bearer for edge-gated requests. When the authentik
/// client_credentials config is fully present, exchange it for a short-lived JWT
/// (locally cached); otherwise fall back to the static `--api-key` (transitional),
/// or no auth at all — on-box localhost needs none.
async fn resolve_edge_bearer(
    token_url: Option<&str>,
    client_id: Option<&str>,
    username: Option<&str>,
    password: Option<&str>,
    api_key: Option<&str>,
) -> Result<Option<String>> {
    if let (Some(url), Some(cid), Some(user), Some(pw)) = (token_url, client_id, username, password)
    {
        return Ok(Some(edge_jwt_cached(url, cid, user, pw).await?));
    }
    Ok(api_key.map(str::to_string))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Return a valid edge JWT, reusing a cached one until ~60s before it expires and
/// otherwise fetching a fresh one via client_credentials. Cache errors are
/// non-fatal — a bad/missing cache just means a fresh fetch.
async fn edge_jwt_cached(
    token_url: &str,
    client_id: &str,
    username: &str,
    password: &str,
) -> Result<String> {
    let now = now_secs();
    let path = edge_token_cache_path(token_url, client_id, username);
    if let Some(p) = &path
        && let Ok(bytes) = std::fs::read(p)
        && let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes)
        && let Some(token) = v.get("access_token").and_then(|t| t.as_str())
        && v.get("expires_at").and_then(|t| t.as_u64()).unwrap_or(0) > now + 60
    {
        return Ok(token.to_string());
    }
    let (access_token, expires_in) =
        fetch_edge_jwt(token_url, client_id, username, password).await?;
    if let Some(p) = &path {
        let v = serde_json::json!({ "access_token": access_token, "expires_at": now + expires_in });
        let _ = write_token_cache(p, &v);
    }
    Ok(access_token)
}

/// One `grant_type=client_credentials` exchange against authentik's token
/// endpoint. Returns `(access_token, expires_in_secs)`.
async fn fetch_edge_jwt(
    token_url: &str,
    client_id: &str,
    username: &str,
    password: &str,
) -> Result<(String, u64)> {
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .context("build token client")?;
    let body = form_urlencode(&[
        ("grant_type", "client_credentials"),
        ("client_id", client_id),
        ("username", username),
        ("password", password),
        ("scope", "openid"),
    ]);
    let response = client
        .post(token_url)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(body)
        .send()
        .await
        .with_context(|| format!("POST {token_url}"))?;
    let status = response.status();
    let text = response.text().await.context("read token response")?;
    if !status.is_success() {
        let detail: String = text.chars().take(200).collect();
        anyhow::bail!("edge token endpoint {token_url} -> {status}: {detail}");
    }
    let value: serde_json::Value = serde_json::from_str(&text).context("parse token response")?;
    let access_token = value
        .get("access_token")
        .and_then(|t| t.as_str())
        .context("token response missing access_token")?
        .to_string();
    let expires_in = value
        .get("expires_in")
        .and_then(|t| t.as_u64())
        .unwrap_or(3600);
    Ok((access_token, expires_in))
}

/// Where the edge JWT is cached. `SANTI_TOKEN_CACHE` overrides with an explicit
/// file path; else `$HOME/.cache/santi/edge-jwt-<key>.json`; else the temp dir.
/// `<key>` derives from (token_url, client_id, username) so distinct edges don't
/// collide.
fn edge_token_cache_path(
    token_url: &str,
    client_id: &str,
    username: &str,
) -> Option<std::path::PathBuf> {
    use std::hash::{Hash, Hasher};
    if let Ok(explicit) = std::env::var("SANTI_TOKEN_CACHE")
        && !explicit.trim().is_empty()
    {
        return Some(std::path::PathBuf::from(explicit));
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    token_url.hash(&mut hasher);
    client_id.hash(&mut hasher);
    username.hash(&mut hasher);
    let key = format!("{:016x}", hasher.finish());
    let dir = std::env::var("HOME")
        .ok()
        .map(|home| std::path::PathBuf::from(home).join(".cache/santi"))
        .unwrap_or_else(std::env::temp_dir);
    Some(dir.join(format!("edge-jwt-{key}.json")))
}

/// Encode key/value pairs as `application/x-www-form-urlencoded` (RFC 3986
/// unreserved set kept literal, space → `+`, everything else percent-encoded).
fn form_urlencode(pairs: &[(&str, &str)]) -> String {
    fn enc(input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        for byte in input.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(byte as char)
                }
                b' ' => out.push('+'),
                _ => out.push_str(&format!("%{byte:02X}")),
            }
        }
        out
    }
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
        .collect::<Vec<_>>()
        .join("&")
}

fn write_token_cache(path: &std::path::Path, value: &serde_json::Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_vec(value)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

async fn get(client: &reqwest::Client, url: &str) -> Result<()> {
    let response = client
        .get(url)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    print_json(response).await
}

async fn post(client: &reqwest::Client, url: &str, body: Option<serde_json::Value>) -> Result<()> {
    let mut request = client.post(url).timeout(REQUEST_TIMEOUT);
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

/// IM send: POST the message, then (with --reply) poll the participant's inbox
/// for the soul's reply until it arrives or the timeout elapses. Baselines the
/// inbox high-water BEFORE sending so only the NEW reply is shown; on silence it
/// returns (the reply may still arrive later — the caller can poll it).
async fn im_send(
    client: &reqwest::Client,
    base: &str,
    body: serde_json::Value,
    participant: &str,
    reply: bool,
    reply_timeout: u64,
) -> Result<()> {
    let baseline = if reply {
        im_inbox_high_water(client, base, participant).await?
    } else {
        0
    };
    let url = format!("{base}/api/v1/im/send");
    let response = client
        .post(&url)
        .timeout(REQUEST_TIMEOUT)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("POST {url}"))?;
    let status = response.status();
    let text = response.text().await.context("read response body")?;
    if !status.is_success() {
        println!("{text}");
        anyhow::bail!("im send failed with status {status}");
    }
    if !reply {
        match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(value) => println!("{}", serde_json::to_string_pretty(&value)?),
            Err(_) => println!("{text}"),
        }
        return Ok(());
    }
    // Poll for the reply. A real turn runs minutes; poll gently past the baseline.
    let inbox_url = format!("{base}/api/v1/im/inbox/{participant}?since={baseline}");
    let deadline = Instant::now() + Duration::from_secs(reply_timeout);
    loop {
        let entries: serde_json::Value = client
            .get(&inbox_url)
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .with_context(|| format!("GET {inbox_url}"))?
            .json()
            .await
            .context("parse inbox")?;
        if entries
            .as_array()
            .is_some_and(|entries| !entries.is_empty())
        {
            println!("{}", serde_json::to_string_pretty(&entries)?);
            return Ok(());
        }
        if Instant::now() >= deadline {
            eprintln!(
                "(no reply within {reply_timeout}s — it may still arrive; poll: santi im poll --as {participant} --since {baseline})"
            );
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// The current max `seq` in a participant's inbox — the baseline a `--reply` send
/// polls past, so it shows only the new reply, not the conversation history.
async fn im_inbox_high_water(
    client: &reqwest::Client,
    base: &str,
    participant: &str,
) -> Result<i64> {
    let url = format!("{base}/api/v1/im/inbox/{participant}?since=0");
    let entries: serde_json::Value = client
        .get(&url)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .json()
        .await
        .context("parse inbox")?;
    Ok(entries
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("seq").and_then(serde_json::Value::as_i64))
        .max()
        .unwrap_or(0))
}

/// POST a send, then optionally `--watch` the stream until the strand is
/// idle again. Without `--watch` this is the prior fire-and-return behavior.
async fn send(
    client: &reqwest::Client,
    base: &str,
    strand_id: &str,
    body: serde_json::Value,
    watch: bool,
) -> Result<()> {
    let url = format!("{base}/api/v1/strands/{strand_id}/send");
    // The send itself returns as soon as the message is enqueued (it does not
    // wait for the turn), so it is an immediate request and gets the timeout;
    // the optional `--watch` below follows the SSE stream, which does not.
    let response = client
        .post(&url)
        .timeout(REQUEST_TIMEOUT)
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
    watch_until_idle(client, base, strand_id, seed_turn).await
}

/// Follow the strand's SSE stream, tracking which turns are in flight, and
/// return once none remain (the strand has caught up). Each event is
/// relayed to stdout as one compact JSON line; the client models only the
/// turn-lifecycle events it needs to decide "idle", nothing more.
async fn watch_until_idle(
    client: &reqwest::Client,
    base: &str,
    strand_id: &str,
    seed_turn: Option<String>,
) -> Result<()> {
    let url = format!("{base}/api/v1/strands/{strand_id}/events");
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

    fn defaults(strand: Option<&str>, soul: Option<&str>) -> ClientDefaults {
        ClientDefaults {
            strand: strand.map(str::to_string),
            soul: soul.map(str::to_string),
        }
    }

    #[test]
    fn resolve_strand_prefers_explicit_then_default() {
        let d = defaults(Some("sess_default"), None);
        assert_eq!(d.resolve_strand(Some("sess_x".into())).unwrap(), "sess_x");
        assert_eq!(d.resolve_strand(None).unwrap(), "sess_default");
    }

    #[test]
    fn resolve_strand_errors_when_both_empty() {
        assert!(defaults(None, None).resolve_strand(None).is_err());
        // A blank default is treated as absent, not a valid id.
        assert!(defaults(Some("  "), None).resolve_strand(None).is_err());
    }

    #[test]
    fn soul_blank_is_none() {
        assert_eq!(defaults(None, Some("soul_x")).soul(), Some("soul_x"));
        assert_eq!(defaults(None, Some("   ")).soul(), None);
        assert_eq!(defaults(None, None).soul(), None);
    }

    #[test]
    fn form_urlencode_encodes_reserved_chars() {
        assert_eq!(
            form_urlencode(&[("grant_type", "client_credentials"), ("scope", "openid")]),
            "grant_type=client_credentials&scope=openid"
        );
        // reserved / non-unreserved bytes get percent-encoded; space → '+'
        assert_eq!(form_urlencode(&[("k", "a b&c=d/e")]), "k=a+b%26c%3Dd%2Fe");
        // the RFC 3986 unreserved set stays literal
        assert_eq!(form_urlencode(&[("x", "Az0-_.~")]), "x=Az0-_.~");
    }

    #[test]
    fn split_send_args_two_then_one() {
        let d = defaults(Some("sess_default"), None);
        let (id, text) = split_send_args(vec!["sess_x".into(), "hi".into()], &d).unwrap();
        assert_eq!((id.as_str(), text.as_str()), ("sess_x", "hi"));

        let (id, text) = split_send_args(vec!["hello".into()], &d).unwrap();
        assert_eq!((id.as_str(), text.as_str()), ("sess_default", "hello"));

        // One arg with no default strand is a usage error, not a silent send.
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
