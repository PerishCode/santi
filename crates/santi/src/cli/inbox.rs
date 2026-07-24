use clap::Subcommand;

#[derive(Subcommand)]
pub enum InboxCommand {
    #[command(
        about = "Enqueue one `santi_system` record into a strand's durable inbox WITHOUT a running service (a direct MQ producer). The strand comes from --strand/SANTI_STRAND_ID and must already exist"
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
