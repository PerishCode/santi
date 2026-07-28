use clap::Parser;
use santi::cli::{
    Cli, Command, CompactCommand, EffectCommand, EffectOutcomeArg, Job, StrandCommand, Strategy,
    Stream, Turn, WatchFormat, Webhook,
};

#[test]
fn errors() {
    let parsed = Cli::try_parse_from([
        "santi",
        "errors",
        "--scope-kind",
        "runtime",
        "--scope-id",
        "default",
        "--limit",
        "12",
    ])
    .unwrap();
    let Command::Errors {
        scope_kind,
        scope_id,
        limit,
    } = parsed.command
    else {
        panic!("expected global errors command");
    };
    assert_eq!(scope_kind, "runtime");
    assert_eq!(scope_id, "default");
    assert_eq!(limit, 12);

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
fn receipt() {
    let parsed = Cli::try_parse_from(["santi", "receipt", "inbox_123"]).unwrap();
    let Command::Receipt { inbox } = parsed.command else {
        panic!("expected receipt command");
    };
    assert_eq!(inbox, "inbox_123");
}

#[test]
fn turns() {
    let parsed = Cli::try_parse_from(["santi", "turn", "stop", "turn_123"]).unwrap();
    let Command::Turn(Turn::Stop { id }) = parsed.command else {
        panic!("expected turn stop command");
    };
    assert_eq!(id, "turn_123");
}

#[test]
fn jobs() {
    let parsed = Cli::try_parse_from([
        "santi",
        "job",
        "create",
        "compile release",
        "cargo build --release",
        "--cwd",
        "strand://repo",
        "--timeout-seconds",
        "90",
        "--output-limit-bytes",
        "4096",
        "--remind-every-seconds",
        "30",
    ])
    .unwrap();
    let Command::Job(Job::Create {
        description,
        command,
        cwd,
        timeout_seconds,
        output_limit_bytes,
        remind_every_seconds,
    }) = parsed.command
    else {
        panic!("expected job create");
    };
    assert_eq!(description, "compile release");
    assert_eq!(command, "cargo build --release");
    assert_eq!(cwd.as_deref(), Some("strand://repo"));
    assert_eq!(timeout_seconds, Some(90));
    assert_eq!(output_limit_bytes, Some(4096));
    assert_eq!(remind_every_seconds, Some(30));

    let parsed = Cli::try_parse_from([
        "santi", "job", "logs", "job_1", "--stream", "stderr", "--cursor", "12", "--limit", "80",
    ])
    .unwrap();
    let Command::Job(Job::Logs {
        id,
        stream,
        cursor,
        limit,
    }) = parsed.command
    else {
        panic!("expected job logs");
    };
    assert_eq!(id, "job_1");
    assert!(matches!(stream, Stream::Stderr));
    assert_eq!(cursor, "12");
    assert_eq!(limit, 80);
}

#[test]
fn webhooks() {
    let parsed = Cli::try_parse_from(["santi", "webhook", "list"]).unwrap();
    assert!(matches!(parsed.command, Command::Webhook(Webhook::List)));

    let parsed = Cli::try_parse_from([
        "santi",
        "webhook",
        "ensure",
        "secretary",
        "--adaptor",
        "github",
        "--soul",
        "soul_default",
        "--credential",
        "SANTI_WEBHOOK_GITHUB_SECRET",
    ])
    .unwrap();
    let Command::Webhook(Webhook::Ensure {
        name,
        adaptor,
        soul,
        strategy,
        credential,
    }) = parsed.command
    else {
        panic!("expected webhook ensure command");
    };
    assert_eq!(name, "secretary");
    assert_eq!(adaptor, "github");
    assert_eq!(soul, "soul_default");
    assert_eq!(strategy, Strategy::Thread);
    assert_eq!(credential, "SANTI_WEBHOOK_GITHUB_SECRET");
}

#[test]
fn effects() {
    let parsed = Cli::try_parse_from(["santi", "effect", "query", "effect_123"]).unwrap();
    let Command::Effect(EffectCommand::Query { effect }) = parsed.command else {
        panic!("expected effect query");
    };
    assert_eq!(effect, "effect_123");

    let parsed = Cli::try_parse_from([
        "santi",
        "effect",
        "resolve",
        "effect_123",
        "--outcome",
        "not-applied",
        "--evidence",
        "marker absent",
    ])
    .unwrap();
    let Command::Effect(EffectCommand::Resolve {
        effect,
        outcome,
        evidence,
    }) = parsed.command
    else {
        panic!("expected effect resolution");
    };
    assert_eq!(effect, "effect_123");
    assert_eq!(outcome, EffectOutcomeArg::NotApplied);
    assert_eq!(evidence, "marker absent");
}

#[test]
fn watch() {
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
fn capsule() {
    let parsed = Cli::try_parse_from([
        "santi",
        "compact",
        "capsule",
        "--from",
        "1",
        "--to",
        "9",
        "--summary-file",
        "summary.md",
        "--source",
        "operator",
        "--reason",
        "recover budget",
        "--risk",
        "summary may omit detail",
        "--dry",
    ])
    .unwrap();
    let Command::Compact(CompactCommand::Capsule {
        from,
        to,
        summary_file,
        source,
        reason,
        risk,
        dry,
        ..
    }) = parsed.command
    else {
        panic!("expected compact capsule command");
    };
    assert_eq!(from, Some(1));
    assert_eq!(to, Some(9));
    assert_eq!(summary_file.as_deref(), Some("summary.md"));
    assert_eq!(source, "operator");
    assert_eq!(reason, "recover budget");
    assert_eq!(risk, "summary may omit detail");
    assert!(dry);
}
