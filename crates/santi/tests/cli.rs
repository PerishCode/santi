use clap::Parser;
use santi::auth::form_urlencode;
use santi::cli::{Cli, ClientDefaults, Command, StrandCommand, split_send_args};

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
fn parses_drive() {
    let parsed = Cli::try_parse_from(["santi", "strand", "drive", "ss_blocked"]).unwrap();
    let Command::Strand(StrandCommand::Drive { id }) = parsed.command else {
        panic!("expected strand drive command");
    };
    assert_eq!(id.as_deref(), Some("ss_blocked"));

    let parsed =
        Cli::try_parse_from(["santi", "--strand", "ss_default", "strand", "drive"]).unwrap();
    let Command::Strand(StrandCommand::Drive { id }) = parsed.command else {
        panic!("expected strand drive command");
    };
    assert_eq!(id, None);
    assert_eq!(parsed.strand.as_deref(), Some("ss_default"));
}

#[test]
fn parses_doctor() {
    let parsed = Cli::try_parse_from(["santi", "doctor"]).unwrap();
    assert!(matches!(parsed.command, Command::Doctor));
}

#[path = "cli/parse.rs"]
mod parse;
