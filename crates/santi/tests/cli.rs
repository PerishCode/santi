use clap::Parser;
use santi::auth::form_urlencode;
use santi::cli::{
    Cli, ClientDefaults, Command, CompactCommand, StrandCommand, WatchFormat, split_send_args,
};

fn defaults(strand: Option<&str>, soul: Option<&str>) -> ClientDefaults {
    ClientDefaults {
        strand: strand.map(str::to_string),
        soul: soul.map(str::to_string),
    }
}

#[test]
fn defaults_resolve_strand() {
    let defaults = defaults(Some("sess_default"), None);
    assert_eq!(
        defaults.resolve_strand(Some("sess_x".into())).unwrap(),
        "sess_x"
    );
    assert_eq!(defaults.resolve_strand(None).unwrap(), "sess_default");
}

#[test]
fn defaults_require_strand() {
    assert!(defaults(None, None).resolve_strand(None).is_err());
    assert!(defaults(Some("  "), None).resolve_strand(None).is_err());
}

#[test]
fn defaults_trim_soul() {
    assert_eq!(defaults(None, Some("soul_x")).soul(), Some("soul_x"));
    assert_eq!(defaults(None, Some("   ")).soul(), None);
    assert_eq!(defaults(None, None).soul(), None);
}

#[test]
fn form_encodes() {
    assert_eq!(
        form_urlencode(&[("grant_type", "client_credentials"), ("scope", "openid")]),
        "grant_type=client_credentials&scope=openid"
    );
    assert_eq!(form_urlencode(&[("k", "a b&c=d/e")]), "k=a+b%26c%3Dd%2Fe");
    assert_eq!(form_urlencode(&[("x", "Az0-_.~")]), "x=Az0-_.~");
}

#[test]
fn send_args_split() {
    let config = defaults(Some("sess_default"), None);
    let (id, text) = split_send_args(vec!["sess_x".into(), "hi".into()], &config).unwrap();
    assert_eq!((id.as_str(), text.as_str()), ("sess_x", "hi"));

    let (id, text) = split_send_args(vec!["hello".into()], &config).unwrap();
    assert_eq!((id.as_str(), text.as_str()), ("sess_default", "hello"));
    assert!(split_send_args(vec!["hello".into()], &defaults(None, None)).is_err());
}

#[test]
fn parses_fork() {
    let parsed = Cli::try_parse_from(["santi", "strand", "fork", "ss_parent"]).unwrap();
    let Command::Strand(StrandCommand::Fork { id }) = parsed.command else {
        panic!("expected strand fork command");
    };
    assert_eq!(id.as_deref(), Some("ss_parent"));

    let parsed = Cli::try_parse_from(["santi", "strand", "fork"]).unwrap();
    let Command::Strand(StrandCommand::Fork { id }) = parsed.command else {
        panic!("expected strand fork command");
    };
    assert_eq!(id, None);
}

#[test]
fn errors_replaces_rejections() {
    let parsed =
        Cli::try_parse_from(["santi", "strand", "errors", "ss_1", "--limit", "12"]).unwrap();
    let Command::Strand(StrandCommand::Errors { id, limit }) = parsed.command else {
        panic!("expected strand errors command");
    };
    assert_eq!(id.as_deref(), Some("ss_1"));
    assert_eq!(limit, 12);
    assert!(Cli::try_parse_from(["santi", "strand", "rejections", "ss_1"]).is_err());
}

#[test]
fn parses_watch_flags() {
    let parsed = Cli::try_parse_from(["santi", "strand", "send", "--watch", "hello"]).unwrap();
    let Command::Strand(StrandCommand::Send {
        args,
        watch,
        watch_format,
    }) = parsed.command
    else {
        panic!("expected strand send command");
    };
    assert_eq!(args, vec!["hello"]);
    assert!(watch);
    assert_eq!(watch_format, WatchFormat::Filtered);

    assert!(
        Cli::try_parse_from(["santi", "strand", "send", "--watch-format", "raw", "hello"]).is_err()
    );

    let parsed = Cli::try_parse_from([
        "santi",
        "strand",
        "send",
        "--watch",
        "--watch-format",
        "raw",
        "hello",
    ])
    .unwrap();
    let Command::Strand(StrandCommand::Send { watch_format, .. }) = parsed.command else {
        panic!("expected strand send command");
    };
    assert_eq!(watch_format, WatchFormat::Raw);

    let parsed = Cli::try_parse_from(["santi", "strand", "events", "ss_1"]).unwrap();
    let Command::Strand(StrandCommand::Events { id, format }) = parsed.command else {
        panic!("expected strand events command");
    };
    assert_eq!(id.as_deref(), Some("ss_1"));
    assert_eq!(format, WatchFormat::Raw);

    let parsed =
        Cli::try_parse_from(["santi", "strand", "events", "ss_1", "--format", "filtered"]).unwrap();
    let Command::Strand(StrandCommand::Events { format, .. }) = parsed.command else {
        panic!("expected strand events command");
    };
    assert_eq!(format, WatchFormat::Filtered);
}

#[test]
fn parses_capsule() {
    let parsed = Cli::try_parse_from([
        "santi",
        "compact",
        "capsule",
        "--from-seq",
        "1",
        "--to-seq",
        "9",
        "--summary-file",
        "summary.md",
        "--source",
        "operator",
        "--reason",
        "recover budget",
        "--risk",
        "summary may omit detail",
        "--dry-run",
    ])
    .unwrap();
    let Command::Compact(CompactCommand::Capsule {
        from_seq,
        to_seq,
        summary_file,
        source,
        reason,
        risk,
        dry_run,
        ..
    }) = parsed.command
    else {
        panic!("expected compact capsule command");
    };
    assert_eq!(from_seq, Some(1));
    assert_eq!(to_seq, Some(9));
    assert_eq!(summary_file.as_deref(), Some("summary.md"));
    assert_eq!(source, "operator");
    assert_eq!(reason, "recover budget");
    assert_eq!(risk, "summary may omit detail");
    assert!(dry_run);
}
