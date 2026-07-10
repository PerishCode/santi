use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:43307";

#[derive(Parser)]
#[command(
    name = "santi",
    version,
    about = "santi runtime: server (`service`) and HTTP client"
)]
pub struct Cli {
    /// Base URL of a running santi server. Falls back to SANTI_API_URL, then
    /// the local default. Only used by the HTTP client commands.
    #[arg(long, global = true, env = "SANTI_API_URL", default_value = DEFAULT_BASE_URL)]
    pub base_url: String,

    /// Static bearer token sent on client requests. Falls back to SANTI_API_KEY.
    /// Transitional: santi itself no longer gates on this; prefer the edge-auth
    /// (authentik client_credentials) flags below to reach santi behind forward-auth.
    #[arg(long, global = true, env = "SANTI_API_KEY")]
    pub api_key: Option<String>,

    /// Edge auth via authentik client_credentials. When token-url, client-id,
    /// username AND password are all set, the client exchanges them for a
    /// short-lived JWT (cached locally, ~1h) and sends THAT as the bearer instead
    /// of --api-key — the way to reach santi behind authentik forward-auth.
    #[arg(long, global = true, env = "SANTI_AUTH_TOKEN_URL")]
    pub auth_token_url: Option<String>,
    #[arg(long, global = true, env = "SANTI_AUTH_CLIENT_ID")]
    pub auth_client_id: Option<String>,
    #[arg(long, global = true, env = "SANTI_AUTH_USERNAME")]
    pub auth_username: Option<String>,
    #[arg(long, global = true, env = "SANTI_AUTH_PASSWORD")]
    pub auth_password: Option<String>,

    /// Default strand id used when a strand subcommand omits an explicit id.
    /// Falls back to SANTI_STRAND_ID. Empty/absent → an id must be passed.
    #[arg(long, global = true, env = "SANTI_STRAND_ID")]
    pub strand: Option<String>,

    /// Default soul addressed by `strand send`. Falls back to SANTI_SOUL_ID.
    /// Empty/absent → the runtime's default soul (the pre-multi-soul path).
    #[arg(long, global = true, env = "SANTI_SOUL_ID")]
    pub soul: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Run the runtime server in-process (`serve`, `export-openapi`).
    Service {
        /// Arguments forwarded to the server (e.g. `serve`, `export-openapi`,
        /// `--config`, `--provider`).
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Offline pre-check of the store, default soul memory, and provider budget.
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
        /// Internal final-version handover invoked by the upgrade runner.
        #[arg(long, hide = true, conflicts_with = "run")]
        finalize: bool,
    },
    /// GET /api/v1/health
    Health,
    /// Query canonical incidents by error scope.
    Errors {
        #[arg(long, default_value = "runtime")]
        scope_kind: String,
        #[arg(long, default_value = "default")]
        scope_id: String,
        #[arg(long, default_value_t = 50)]
        limit: i64,
    },
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
pub enum ImCommand {
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
        /// The reply text. For multi-line or shell-sensitive content, prefer
        /// --file or --stdin.
        #[arg(
            conflicts_with_all = ["file", "stdin"],
            required_unless_present_any = ["file", "stdin"]
        )]
        text: Option<String>,
        /// Read the reply text from a file (or `-` for stdin).
        #[arg(long, value_name = "PATH", conflicts_with_all = ["text", "stdin"])]
        file: Option<String>,
        /// Read the reply text from stdin.
        #[arg(long, conflicts_with_all = ["text", "file"])]
        stdin: bool,
    },
}

#[derive(Subcommand)]
pub enum CompactCommand {
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
    /// Create a provider-visible compact capsule with provenance/risk metadata.
    Capsule {
        /// First message of the range (a fixed user/assistant message id).
        #[arg(long, conflicts_with = "from_seq")]
        from: Option<String>,
        /// Last message of the range (a fixed user/assistant message id).
        #[arg(long, conflicts_with = "to_seq")]
        to: Option<String>,
        /// First message seq of the range.
        #[arg(long, conflicts_with = "from")]
        from_seq: Option<i64>,
        /// Last message seq of the range.
        #[arg(long, conflicts_with = "to")]
        to_seq: Option<i64>,
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
        /// Who/what authored this capsule decision.
        #[arg(long, default_value = "operator")]
        source: String,
        /// Why this range is being compressed.
        #[arg(long)]
        reason: String,
        /// Known risk or lossiness in the summary.
        #[arg(long)]
        risk: String,
        /// How to inspect the original covered range.
        #[arg(
            long,
            default_value = "original entries remain queryable with compact query"
        )]
        queryability: String,
        /// Validate and preview the capsule plan without writing a compact.
        #[arg(long)]
        dry_run: bool,
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
pub enum InboxCommand {
    /// Enqueue one `santi_system` record into a strand's durable inbox WITHOUT a
    /// running service (a direct MQ producer). The strand comes from
    /// --strand/SANTI_STRAND_ID and must already exist. Used by the self-upgrade
    /// flow to seed the "come look" record before starting the final version.
    Seed {
        /// The message text (the "come look" occurrence). For multi-line or
        /// shell-sensitive content, prefer --file or --stdin.
        #[arg(
            conflicts_with_all = ["file", "stdin"],
            required_unless_present_any = ["file", "stdin"]
        )]
        text: Option<String>,
        /// Read the message text from a file (or `-` for stdin).
        #[arg(long, value_name = "PATH", conflicts_with_all = ["text", "stdin"])]
        file: Option<String>,
        /// Read the message text from stdin.
        #[arg(long, conflicts_with_all = ["text", "file"])]
        stdin: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum WatchFormat {
    /// Human-readable milestone lines that omit high-volume stream chunks.
    Filtered,
    /// Raw debugging output: SSE bytes for `strand events`, JSON event data for `send --watch`.
    Raw,
}

#[derive(Subcommand)]
pub enum StrandCommand {
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
    /// GET /api/v1/strands/{id}/budget (id falls back to --strand)
    Budget { id: Option<String> },
    /// GET /api/v1/strands/{id}/errors (id falls back to --strand)
    Errors {
        id: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: i64,
    },
    /// POST /api/v1/strands/{id}/fork (id falls back to --strand)
    Fork { id: Option<String> },
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
        /// Output format for --watch. `raw` preserves the prior JSON-line debug stream.
        #[arg(
            long = "watch-format",
            value_enum,
            default_value_t = WatchFormat::Filtered,
            requires = "watch"
        )]
        watch_format: WatchFormat,
    },
    /// GET /api/v1/strands/{id}/events — follows the SSE stream (id falls back
    /// to --strand). Runs until interrupted; use `send --watch` to stop on idle.
    Events {
        id: Option<String>,
        /// Output format. Raw preserves the prior SSE byte stream.
        #[arg(long, value_enum, default_value_t = WatchFormat::Raw)]
        format: WatchFormat,
    },
}

/// Client-side defaults resolved from global flags or environment variables.
/// They only fill omitted request fields and are not runtime concepts.
pub struct ClientDefaults {
    pub strand: Option<String>,
    pub soul: Option<String>,
}

impl ClientDefaults {
    /// Resolve a strand id: an explicit positional wins, else the default.
    /// Both empty is a usage error — same "you must name a strand" path as before.
    pub fn resolve_strand(&self, explicit: Option<String>) -> Result<String> {
        explicit
            .or_else(|| self.strand.clone())
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!("no strand id: pass one or set --strand / SANTI_STRAND_ID")
            })
    }

    /// The soul to address, or None to let the runtime use its default soul.
    pub fn soul(&self) -> Option<&str> {
        self.soul
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }
}

/// Split `send` positionals into `(strand_id, text)`.
pub fn split_send_args(
    mut args: Vec<String>,
    defaults: &ClientDefaults,
) -> Result<(String, String)> {
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
