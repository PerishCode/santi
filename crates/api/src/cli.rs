use clap::{Parser, Subcommand};

use crate::config::ConfigArgs;

#[derive(Parser)]
#[command(
    name = "santi-api",
    version = plumb::version!("SANTI"),
    about = "santi runtime API server and local operator"
)]
pub struct Cli {
    #[arg(long, global = true)]
    pub config: Option<String>,
    #[arg(long, global = true, env = "SANTI_STRAND_ID")]
    pub strand: Option<String>,
    #[command(flatten)]
    pub over: ConfigArgs,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    Serve,
    #[command(name = "export-openapi")]
    Export,
    #[command(
        about = "Check the store, default soul memory, and provider budget without a running server"
    )]
    Doctor {
        #[arg(
            help = "Internal storage-only check used by deployment and recovery",
            long,
            hide = true
        )]
        storage_only: bool,
    },
    #[command(about = "Read durable tool activity from the local Keel estate")]
    Audit {
        #[arg(long)]
        turn: Option<String>,
        #[arg(long)]
        failed: bool,
        #[arg(short = 'n', long = "limit", default_value_t = 30)]
        limit: usize,
        #[arg(long, hide = true)]
        after: Option<String>,
    },
    #[command(about = "Operate directly on local runtime state")]
    #[command(subcommand)]
    Inbox(InboxCommand),
    #[command(about = "Inspect configured runtime capability authority")]
    #[command(subcommand)]
    Capability(Capability),
    #[command(name = "__job", hide = true)]
    #[command(subcommand)]
    Job(Job),
}

#[derive(Subcommand)]
pub enum Capability {
    #[command(about = "Print the active key id and Ed25519 public key")]
    Public,
}

#[derive(Subcommand)]
pub enum Job {
    Run,
    Finalize,
}

#[derive(Subcommand)]
pub enum InboxCommand {
    #[command(
        about = "Enqueue one `santi_system` record into a strand's durable inbox without a running server"
    )]
    Seed {
        #[arg(
            help = "The message text. For multi-line or shell-sensitive content, prefer --file or --stdin"
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
