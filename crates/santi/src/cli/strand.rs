use clap::Subcommand;

use super::*;

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
