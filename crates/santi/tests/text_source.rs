use clap::Parser;
use santi::cli::{Cli, Command, ImCommand, InboxCommand};
use santi::text_source::{read_im_reply_text, read_inbox_seed_text};

#[test]
fn im_reply_accepts_source() {
    let parsed = Cli::try_parse_from(["santi", "im", "reply", "hello"]).unwrap();
    let Command::Im(ImCommand::Reply { text, file, stdin }) = parsed.command else {
        panic!("expected im reply command");
    };
    assert_eq!(text.as_deref(), Some("hello"));
    assert_eq!(file, None);
    assert!(!stdin);

    let parsed = Cli::try_parse_from(["santi", "im", "reply", "--file", "reply.txt"]).unwrap();
    let Command::Im(ImCommand::Reply { text, file, stdin }) = parsed.command else {
        panic!("expected im reply command");
    };
    assert_eq!(text, None);
    assert_eq!(file.as_deref(), Some("reply.txt"));
    assert!(!stdin);

    let parsed = Cli::try_parse_from(["santi", "im", "reply", "--stdin"]).unwrap();
    let Command::Im(ImCommand::Reply { text, file, stdin }) = parsed.command else {
        panic!("expected im reply command");
    };
    assert_eq!(text, None);
    assert_eq!(file, None);
    assert!(stdin);
}

#[test]
fn im_reply_rejects_sources() {
    assert!(Cli::try_parse_from(["santi", "im", "reply"]).is_err());
    assert!(Cli::try_parse_from(["santi", "im", "reply", "hello", "--stdin"]).is_err());
    assert!(
        Cli::try_parse_from(["santi", "im", "reply", "--file", "reply.txt", "--stdin"]).is_err()
    );
}

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
        read_im_reply_text(Some("hello".into()), None, false).unwrap(),
        "hello"
    );
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
    let expected = "multi\nline `reply`\nand seed\n";
    std::fs::write(&path, expected).unwrap();
    let path = path.to_string_lossy().into_owned();
    let reply = read_im_reply_text(None, Some(path.clone()), false).unwrap();
    let seed = read_inbox_seed_text(None, Some(path.clone()), false).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(reply, expected);
    assert_eq!(seed, expected);
}
