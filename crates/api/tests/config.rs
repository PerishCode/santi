use std::io::Write as _;

use api::config::{Config, ConfigPartial};
use plumb::config::Cascade as _;

fn read(text: &str) -> Config {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(text.as_bytes()).unwrap();
    Config::default().merge(plumb::config::load::<ConfigPartial>(file.path()).unwrap())
}

#[test]
fn example() {
    let held = read(include_str!("../../../santi.example.toml"));
    assert_eq!(held.providers.len(), 3);
    assert_eq!(held.server.grace, 30);
    assert_eq!(
        held.jobs.retention().unwrap(),
        std::time::Duration::from_secs(604800)
    );
}

#[test]
fn grace() {
    let held = read(
        r#"
[server]
grace = 0
"#,
    );
    assert_eq!(held.server.grace, 0);
}

#[test]
fn listen() {
    let held = read(
        r#"
[listen]
host = "0.0.0.0"
port = 43308
prefix = "/santi"
"#,
    );
    assert_eq!(held.listen.address(), "0.0.0.0:43308");
    assert_eq!(held.listen.prefix, "/santi");
}

#[test]
fn environment() {
    let held = read(
        r#"
[environment]
GLOBAL_LITERAL = "value"
GLOBAL_REFERENCE = "env://HOST_VALUE"
"#,
    );
    assert_eq!(
        held.environment.get("GLOBAL_LITERAL").map(String::as_str),
        Some("value")
    );
    assert_eq!(
        held.environment.get("GLOBAL_REFERENCE").map(String::as_str),
        Some("env://HOST_VALUE")
    );
}

#[test]
fn capability() {
    let held = read(
        r#"
[capability]
issuer = "santi.example"
audience = "stim.reply"
key_id = "key-2026"
private_key = "private-material"
ttl_seconds = 120
"#,
    );
    assert_eq!(held.capability.issuer, "santi.example");
    assert_eq!(held.capability.audience, "stim.reply");
    assert_eq!(held.capability.key_id, "key-2026");
    let shown = format!("{:?}", held.capability);
    assert!(shown.contains("[redacted]"));
    assert!(!shown.contains("private-material"));
}

#[test]
fn legacy() {
    let held = read(
        r#"
provider = "old"

[providers.old]
kind = "openai_responses"
api_key = "key"
model = "model"
reasoning_summary = "auto"
input_budget_bytes = 120000
"#,
    );
    let profile = held.providers.get("old").unwrap().resolve("old").unwrap();
    assert_eq!(profile.bytes(), 120000);
}

#[test]
fn unknown() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(
        br#"
[providers.bad]
kind = "chat_completions"
api_key = "key"
model = "model"
base_url = "http://127.0.0.1:1"
bytez = 120000
"#,
    )
    .unwrap();
    assert!(plumb::config::load::<ConfigPartial>(file.path()).is_err());
}

#[test]
fn retention() {
    let held = read(
        r#"
[jobs]
acknowledged_retention_seconds = 0
"#,
    );
    assert!(held.jobs.retention().is_err());
}
