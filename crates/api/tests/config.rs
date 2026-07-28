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
