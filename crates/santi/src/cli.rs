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

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum WatchFormat {
    #[value(help = "Human-readable milestone lines that omit high-volume stream chunks")]
    Filtered,
    #[value(
        help = "Raw debugging output: SSE bytes for `strand events`, JSON event data for `send --watch`"
    )]
    Raw,
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

mod compact;
mod inbox;
mod strand;
pub use compact::*;
pub use inbox::*;
pub use strand::*;
