use clap::Parser;
use santi::cli::{Cli, Command, InboxCommand};
use santi::read_inbox_seed_text;

#[test]
fn inbox_seed_accepts_source() {
    let parsed = Cli::try_parse_from(["santi", "inbox", "seed", "hello"]).unwrap();
    let Command::Inbox(InboxCommand::Seed { text, file, stdin }) = parsed.command else {
        panic!("expected inbox seed command");
    };
    assert_eq!(text.as_deref(), Some("hello"));
    assert_eq!(file, None);
    assert!(!stdin);

    let parsed = Cli::try_parse_from(["santi", "inbox", "seed", "--file", "seed.txt"]).unwrap();
    let Command::Inbox(InboxCommand::Seed { text, file, stdin }) = parsed.command else {
        panic!("expected inbox seed command");
    };
    assert_eq!(text, None);
    assert_eq!(file.as_deref(), Some("seed.txt"));
    assert!(!stdin);

    let parsed = Cli::try_parse_from(["santi", "inbox", "seed", "--stdin"]).unwrap();
    let Command::Inbox(InboxCommand::Seed { text, file, stdin }) = parsed.command else {
        panic!("expected inbox seed command");
    };
    assert_eq!(text, None);
    assert_eq!(file, None);
    assert!(stdin);
}

#[test]
fn inbox_seed_rejects_sources() {
    assert!(Cli::try_parse_from(["santi", "inbox", "seed"]).is_err());
    assert!(Cli::try_parse_from(["santi", "inbox", "seed", "hello", "--stdin"]).is_err());
    assert!(
        Cli::try_parse_from(["santi", "inbox", "seed", "--file", "seed.txt", "--stdin"]).is_err()
    );
}

#[test]
fn reads_text_sources() {
    assert_eq!(
        read_inbox_seed_text(Some("come look".into()), None, false).unwrap(),
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
    let seed = read_inbox_seed_text(None, Some(path.clone()), false).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(seed, expected);
}
