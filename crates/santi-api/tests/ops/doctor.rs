use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use santi_api::config::{Profile, RuntimePaths};
use santi_api::runtime::Runtime;

#[test]
fn reports_budget() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    santi_core::Store::open(&paths.database).expect("open store");
    let held = runtime_under(temp.path(), Some(120000));

    let report = paths.doctor_configured(&held).expect("doctor");
    assert!(report.ok, "expected healthy: {report:?}");
    let provider = report.provider.expect("provider report");
    assert_eq!(provider.profile.as_deref(), Some("openai"));
    assert_eq!(provider.kind.as_deref(), Some("openai_responses"));
    assert_eq!(provider.model.as_deref(), Some("gpt-5.5"));
    assert_eq!(provider.bytes, Some(120000));
    assert_eq!(provider.source.as_deref(), Some("provider_config"));
}

#[test]
fn rejects_missing_budget() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    santi_core::Store::open(&paths.database).expect("open store");
    let held = runtime_under(temp.path(), None);

    let report = paths.doctor_configured(&held).expect("doctor");
    assert!(!report.ok);
    let provider = report.provider.expect("provider report");
    assert!(!provider.ok);
    assert_eq!(provider.profile.as_deref(), Some("openai"));
    assert_eq!(
        provider.error.as_deref(),
        Some("provider openai field bytes is required")
    );
}

fn paths_under(root: &Path) -> RuntimePaths {
    RuntimePaths {
        database: root.join("runtime").join("db"),
        runtime: root.join("runtime"),
        execution: root.join("execution"),
    }
}

fn runtime_under(root: &Path, budget: Option<usize>) -> Runtime {
    #[derive(serde::Deserialize)]
    struct File {
        providers: BTreeMap<String, Profile>,
    }
    let budget = budget
        .map(|value| format!("bytes = {value}"))
        .unwrap_or_default();
    let file: File = toml::from_str(&format!(
        r#"
        [providers.openai]
        kind = "openai_responses"
        api_key = "test-key"
        model = "gpt-5.5"
        {budget}
        "#
    ))
    .expect("parse providers");
    Runtime {
        bind: "127.0.0.1:0".to_string(),
        listen_port: 0,
        provider: "openai".to_string(),
        providers: file.providers,
        paths: paths_under(root),
        shutdown_grace: Duration::from_secs(600),
        upgrade_timeout: Duration::from_secs(600),
        finalizer_bin: "/usr/bin/santi".into(),
        handover_soul: None,
        handover_strand: None,
        github_login: None,
        github_allow: None,
        feishu_key: None,
        feishu_allow: None,
        constitution: None,
    }
}
