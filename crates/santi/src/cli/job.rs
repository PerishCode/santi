use clap::{Subcommand, ValueEnum};

#[derive(Subcommand)]
pub enum Job {
    #[command(about = "Create a durable detached job from the current runtime shell invocation")]
    Create {
        description: String,
        command: String,
        #[arg(long)]
        cwd: Option<String>,
        #[arg(long)]
        timeout_seconds: Option<u64>,
        #[arg(long)]
        output_limit_bytes: Option<u64>,
        #[arg(long)]
        remind_every_seconds: Option<u64>,
    },
    #[command(about = "List jobs owned by --soul/SANTI_SOUL_ID")]
    List,
    #[command(about = "Get one soul-owned job")]
    Get { id: String },
    #[command(about = "Cancel one soul-owned job and its process tree")]
    Cancel { id: String },
    #[command(about = "Read one append-only job log stream from a byte cursor")]
    Logs {
        id: String,
        #[arg(long, value_enum, default_value_t = Stream::Stdout)]
        stream: Stream,
        #[arg(long, default_value = "0")]
        cursor: String,
        #[arg(long, default_value_t = 64 * 1024)]
        limit: usize,
    },
    #[command(about = "Acknowledge a terminal job and release retained supervisor state")]
    Ack { id: String },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Stream {
    Stdout,
    Stderr,
}

impl Stream {
    pub fn wire(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }
}
