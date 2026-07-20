use clap::Subcommand;

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
