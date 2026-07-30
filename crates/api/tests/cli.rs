use api::cli::{Capability, Cli, Command, InboxCommand};
use clap::Parser;

#[test]
fn defaults() {
    let parsed = Cli::try_parse_from(["santi-api"]).unwrap();
    assert!(parsed.command.is_none());
}

#[test]
fn local() {
    let parsed = Cli::try_parse_from(["santi-api", "doctor"]).unwrap();
    assert!(matches!(
        parsed.command,
        Some(Command::Doctor {
            storage_only: false
        })
    ));
    let parsed = Cli::try_parse_from(["santi-api", "doctor", "--storage-only"]).unwrap();
    assert!(matches!(
        parsed.command,
        Some(Command::Doctor { storage_only: true })
    ));
    let parsed = Cli::try_parse_from([
        "santi-api",
        "--strand",
        "ss_one",
        "audit",
        "--turn",
        "turn_one",
        "--failed",
        "-n",
        "7",
    ])
    .unwrap();
    assert!(matches!(
        parsed.command,
        Some(Command::Audit {
            turn: Some(ref turn),
            failed: true,
            limit: 7,
            after: None,
        }) if turn == "turn_one"
    ));
    let parsed = Cli::try_parse_from(["santi-api", "capability", "public"]).unwrap();
    assert!(matches!(
        parsed.command,
        Some(Command::Capability(Capability::Public))
    ));
}

#[test]
fn sources() {
    let parsed = Cli::try_parse_from(["santi-api", "inbox", "seed", "hello"]).unwrap();
    let Some(Command::Inbox(InboxCommand::Seed { text, file, stdin })) = parsed.command else {
        panic!("expected inbox seed command");
    };
    assert_eq!(text.as_deref(), Some("hello"));
    assert_eq!(file, None);
    assert!(!stdin);

    assert!(Cli::try_parse_from(["santi-api", "inbox", "seed"]).is_err());
    assert!(Cli::try_parse_from(["santi-api", "inbox", "seed", "hello", "--stdin"]).is_err());
}

#[test]
fn reads() {
    assert_eq!(
        api::text::read(Some("come look".into()), None, false).unwrap(),
        "come look"
    );

    let path = std::env::temp_dir().join(format!(
        "santi-text-source-test-{}-{}.txt",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let expected = "multi\nline content\nand seed\n";
    std::fs::write(&path, expected).unwrap();
    let path = path.to_string_lossy().into_owned();
    let seed = api::text::read(None, Some(path.clone()), false).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(seed, expected);
}
