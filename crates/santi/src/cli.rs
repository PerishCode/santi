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
    #[arg(
        help = "Base URL of a running santi server. Falls back to SANTI_API_URL, then the local default. Only used by the HTTP client commands",
        long,
        global = true,
        env = "SANTI_API_URL",
        default_value = DEFAULT_BASE_URL
    )]
    pub base_url: String,

    #[arg(
        help = "Static bearer token sent on client requests. Falls back to SANTI_API_KEY. Transitional: santi itself no longer gates on this; prefer the edge-auth (authentik client_credentials) flags below to reach santi behind forward-auth",
        long,
        global = true,
        env = "SANTI_API_KEY"
    )]
    pub api_key: Option<String>,

    #[arg(
        help = "Edge auth via authentik client_credentials. When token-url, client-id, username AND password are all set, the client exchanges them for a short-lived JWT (cached locally, ~1h) and sends THAT as the bearer instead of --api-key — the way to reach santi behind authentik forward-auth",
        long,
        global = true,
        env = "SANTI_AUTH_TOKEN_URL"
    )]
    pub auth_token_url: Option<String>,
    #[arg(long, global = true, env = "SANTI_AUTH_CLIENT_ID")]
    pub auth_client_id: Option<String>,
    #[arg(long, global = true, env = "SANTI_AUTH_USERNAME")]
    pub auth_username: Option<String>,
    #[arg(long, global = true, env = "SANTI_AUTH_PASSWORD")]
    pub auth_password: Option<String>,

    #[arg(
        help = "Default strand id used when a strand subcommand omits an explicit id. Falls back to SANTI_STRAND_ID. Empty/absent → an id must be passed",
        long,
        global = true,
        env = "SANTI_STRAND_ID"
    )]
    pub strand: Option<String>,

    #[arg(
        help = "Default soul addressed by `strand send`. Falls back to SANTI_SOUL_ID. Empty/absent → the runtime's default soul (the pre-multi-soul path)",
        long,
        global = true,
        env = "SANTI_SOUL_ID"
    )]
    pub soul: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    #[command(about = "Run the runtime server in-process (`serve`, `export-openapi`)")]
    Service {
        #[arg(
            help = "Arguments forwarded to the server (e.g. `serve`, `export-openapi`, `--config`, `--provider`)",
            trailing_var_arg = true,
            allow_hyphen_values = true
        )]
        args: Vec<String>,
    },
    #[command(
        about = "Offline pre-check of the store, default soul memory, and provider budget. A local ops command (NOT an HTTP client): exits non-zero when unhealthy, so the upgrade flow can gate on it. See PHASE-07"
    )]
    Doctor {
        #[arg(
            help = "Internal storage-only check used by the installed final-version binary during a self-upgrade trial",
            long,
            hide = true
        )]
        storage_only: bool,
    },
    #[command(about = "Offline store-level ops (act directly on the DB, no running service)")]
    #[command(subcommand)]
    Inbox(InboxCommand),
    #[command(
        about = "Self-upgrade (PHASE-07). Without `--run`: launch the detached upgrade unit and return fast with a signal. With `--run`: the orchestration itself (what the shipped `santi-upgrade.service` oneshot unit invokes). Local ops"
    )]
    Upgrade {
        #[arg(help = "The `.deb` to install (path or, later, a downloaded artifact)")]
        deb: Option<String>,
        #[arg(
            help = "Bootstrap artifact for the currently installed package. Required only until the runtime has retained a verified installed manifest",
            long,
            env = "SANTI_PREVIOUS_DEB"
        )]
        previous_deb: Option<String>,
        #[arg(
            help = "Run the orchestration in-process instead of launching the unit",
            long
        )]
        run: bool,
        #[arg(
            help = "Internal final-version handover invoked by the upgrade runner",
            long,
            hide = true,
            conflicts_with = "run"
        )]
        finalize: bool,
    },
    #[command(about = "GET /api/v1/health")]
    Health,
    #[command(about = "Query canonical incidents by error scope")]
    Errors {
        #[arg(long, default_value = "runtime")]
        scope_kind: String,
        #[arg(long, default_value = "default")]
        scope_id: String,
        #[arg(long, default_value_t = 50)]
        limit: i64,
    },
    #[command(about = "Query one durable accepted-message obligation by inbox receipt id")]
    Receipt { inbox_id: String },
    #[command(about = "Query or explicitly resolve one external-effect attempt")]
    #[command(subcommand)]
    Effect(EffectCommand),
    #[command(about = "Strand resources under /api/v1/strands")]
    #[command(subcommand)]
    Strand(StrandCommand),
    #[command(about = "Compact a strand's own timeline, or query a compact's detail")]
    #[command(subcommand)]
    Compact(CompactCommand),
    #[command(
        about = "The plain IM integrated into santi — converse with a soul as a persistent participant (send/poll over HTTP), or the soul's offline reply egress"
    )]
    #[command(subcommand)]
    Im(ImCommand),
}

#[derive(Subcommand)]
pub enum EffectCommand {
    #[command(about = "GET /api/v1/effects/{id}")]
    Query { effect_id: String },
    #[command(
        about = "Resolve an unknown effect from operator-supplied evidence. This never retries a command or changes its receipt/turn state"
    )]
    Resolve {
        effect_id: String,
        #[arg(long, value_enum)]
        outcome: EffectOutcomeArg,
        #[arg(long)]
        evidence: String,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum EffectOutcomeArg {
    Applied,
    NotApplied,
}

impl EffectOutcomeArg {
    pub fn as_api_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::NotApplied => "not_applied",
        }
    }
}

#[derive(Subcommand)]
pub enum ImCommand {
    #[command(
        about = "Send a message to a soul via the integrated IM, as a persistent participant, and (with --reply) wait for the soul's reply. Target soul from --soul/SANTI_SOUL_ID; participant from --as/SANTI_IM_PARTICIPANT"
    )]
    Send {
        #[arg(help = "The message text")]
        text: String,
        #[arg(
            help = "Your persistent participant id (your IM identity + reply address)",
            long = "as",
            env = "SANTI_IM_PARTICIPANT",
            default_value = "operator"
        )]
        participant: String,
        #[arg(
            help = "After sending, poll your inbox until the soul replies (or timeout)",
            long
        )]
        reply: bool,
        #[arg(
            help = "Max seconds to wait for a reply with --reply (turns run minutes)",
            long,
            default_value_t = 300
        )]
        reply_timeout: u64,
    },
    #[command(about = "Poll a participant's IM inbox once — entries past --since (0 = all)")]
    Poll {
        #[arg(
            help = "The participant id whose inbox to read",
            long = "as",
            env = "SANTI_IM_PARTICIPANT",
            default_value = "operator"
        )]
        participant: String,
        #[arg(long, default_value_t = 0)]
        since: i64,
    },
    #[command(
        about = "Send an early reply before the current IM turn completes. Normal final speech is delivered automatically. OFFLINE (a direct store write, no HTTP), scoped by --strand/SANTI_STRAND_ID and deduplicated by SANTI_TURN_ID"
    )]
    Reply {
        #[arg(
            help = "The reply text. For multi-line or shell-sensitive content, prefer --file or --stdin"
        )]
        #[arg(
            conflicts_with_all = ["file", "stdin"],
            required_unless_present_any = ["file", "stdin"]
        )]
        text: Option<String>,
        #[arg(
            help = "Read the reply text from a file (or `-` for stdin)",
            long,
            value_name = "PATH",
            conflicts_with_all = ["text", "stdin"]
        )]
        file: Option<String>,
        #[arg(
            help = "Read the reply text from stdin",
            long,
            conflicts_with_all = ["text", "file"]
        )]
        stdin: bool,
    },
}

#[derive(Subcommand)]
pub enum CompactCommand {
    #[command(
        about = "POST /api/v1/strands/{id}/compact — collapse [from,to] into a summary. Strand from --strand/SANTI_STRAND_ID; soul from --soul/SANTI_SOUL_ID"
    )]
    Exec {
        #[arg(
            help = "First message of the range (a fixed projected message id)",
            long
        )]
        from: String,
        #[arg(
            help = "Last message of the range (a fixed projected message id)",
            long
        )]
        to: String,
        #[arg(help = "The summary text. Mutually exclusive with --summary-file")]
        #[arg(
            long,
            conflicts_with = "summary_file",
            required_unless_present = "summary_file"
        )]
        summary: Option<String>,
        #[arg(
            help = "Read the summary from a file (or `-` for stdin) instead of --summary",
            long
        )]
        summary_file: Option<String>,
    },
    #[command(about = "Create a provider-visible compact capsule with provenance/risk metadata")]
    Capsule {
        #[arg(
            help = "First message of the range (a fixed projected message id)",
            long,
            conflicts_with = "from_seq"
        )]
        from: Option<String>,
        #[arg(
            help = "Last message of the range (a fixed projected message id)",
            long,
            conflicts_with = "to_seq"
        )]
        to: Option<String>,
        #[arg(help = "First message seq of the range", long, conflicts_with = "from")]
        from_seq: Option<i64>,
        #[arg(help = "Last message seq of the range", long, conflicts_with = "to")]
        to_seq: Option<i64>,
        #[arg(help = "The summary text. Mutually exclusive with --summary-file")]
        #[arg(
            long,
            conflicts_with = "summary_file",
            required_unless_present = "summary_file"
        )]
        summary: Option<String>,
        #[arg(
            help = "Read the summary from a file (or `-` for stdin) instead of --summary",
            long
        )]
        summary_file: Option<String>,
        #[arg(
            help = "Who/what authored this capsule decision",
            long,
            default_value = "operator"
        )]
        source: String,
        #[arg(help = "Why this range is being compressed", long)]
        reason: String,
        #[arg(help = "Known risk or lossiness in the summary", long)]
        risk: String,
        #[arg(help = "How to inspect the original covered range")]
        #[arg(
            long,
            default_value = "original entries remain queryable with compact query"
        )]
        queryability: String,
        #[arg(
            help = "Validate and preview the capsule plan without writing a compact",
            long
        )]
        dry_run: bool,
    },
    #[command(about = "GET /api/v1/compacts/{id} — expand a compact's covered range (paginated)")]
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
    #[command(
        about = "Enqueue one `santi_system` record into a strand's durable inbox WITHOUT a running service (a direct MQ producer). The strand comes from --strand/SANTI_STRAND_ID and must already exist. Used by the self-upgrade flow to seed the \"come look\" record before starting the final version"
    )]
    Seed {
        #[arg(
            help = "The message text (the \"come look\" occurrence). For multi-line or shell-sensitive content, prefer --file or --stdin"
        )]
        #[arg(
            conflicts_with_all = ["file", "stdin"],
            required_unless_present_any = ["file", "stdin"]
        )]
        text: Option<String>,
        #[arg(
            help = "Read the message text from a file (or `-` for stdin)",
            long,
            value_name = "PATH",
            conflicts_with_all = ["text", "stdin"]
        )]
        file: Option<String>,
        #[arg(
            help = "Read the message text from stdin",
            long,
            conflicts_with_all = ["text", "file"]
        )]
        stdin: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum WatchFormat {
    #[value(help = "Human-readable milestone lines that omit high-volume stream chunks")]
    Filtered,
    #[value(
        help = "Raw debugging output: SSE bytes for `strand events`, JSON event data for `send --watch`"
    )]
    Raw,
}

#[derive(Subcommand)]
pub enum StrandCommand {
    #[command(about = "POST /api/v1/strands")]
    Create,
    #[command(about = "GET /api/v1/strands")]
    List,
    #[command(about = "GET /api/v1/strands/{id} (id falls back to --strand/SANTI_STRAND_ID)")]
    Get { id: Option<String> },
    #[command(about = "GET /api/v1/strands/{id}/messages (id falls back to --strand)")]
    Messages { id: Option<String> },
    #[command(about = "GET /api/v1/strands/{id}/runtime (id falls back to --strand)")]
    Runtime { id: Option<String> },
    #[command(about = "GET /api/v1/strands/{id}/budget (id falls back to --strand)")]
    Budget { id: Option<String> },
    #[command(about = "GET /api/v1/strands/{id}/errors (id falls back to --strand)")]
    Errors {
        id: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: i64,
    },
    #[command(about = "POST /api/v1/strands/{id}/fork (id falls back to --strand)")]
    Fork { id: Option<String> },
    #[command(
        about = "POST /api/v1/strands/{id}/drive — explicitly redrive pending or failed receipts (id falls back to --strand)"
    )]
    Drive { id: Option<String> },
    #[command(
        about = "POST /api/v1/strands/{id}/send",
        long_about = "POST /api/v1/strands/{id}/send.\n\nPositional forms: `send <id> <text>` or `send <text>` (id then falls back to --strand/SANTI_STRAND_ID). Soul comes from --soul/SANTI_SOUL_ID."
    )]
    Send {
        #[arg(
            help = "Either `<id> <text>` or just `<text>`",
            num_args = 1..=2,
            required = true
        )]
        args: Vec<String>,
        #[arg(
            help = "After sending, follow the stream until the strand goes idle, then exit. Robust to coalescing and silent (speechless) completions",
            long
        )]
        watch: bool,
        #[arg(
            help = "Output format for --watch. `raw` preserves the prior JSON-line debug stream"
        )]
        #[arg(
            long = "watch-format",
            value_enum,
            default_value_t = WatchFormat::Filtered,
            requires = "watch"
        )]
        watch_format: WatchFormat,
    },
    #[command(
        about = "GET /api/v1/strands/{id}/events — follows the SSE stream (id falls back to --strand). Runs until interrupted; use `send --watch` to stop on idle"
    )]
    Events {
        id: Option<String>,
        #[arg(
            help = "Output format. Raw preserves the prior SSE byte stream",
            long,
            value_enum,
            default_value_t = WatchFormat::Raw
        )]
        format: WatchFormat,
    },
}

pub struct ClientDefaults {
    pub strand: Option<String>,
    pub soul: Option<String>,
}

impl ClientDefaults {
    pub fn resolve_strand(&self, explicit: Option<String>) -> Result<String> {
        explicit
            .or_else(|| self.strand.clone())
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!("no strand id: pass one or set --strand / SANTI_STRAND_ID")
            })
    }

    pub fn soul(&self) -> Option<&str> {
        self.soul
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }
}

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
