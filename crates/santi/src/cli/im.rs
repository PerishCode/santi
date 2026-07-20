use clap::Subcommand;

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
