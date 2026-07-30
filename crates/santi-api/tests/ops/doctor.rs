use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use santi_api::config::{Layout, Profile};
use santi_api::runtime::Runtime;

#[tokio::test]
async fn reports() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    let store = super::support::bootstrap(&paths.database).await;
    store
        .seed(santi_core::GENESIS, &santi_core::now())
        .await
        .expect("seed");
    let held = runtime_under(temp.path(), Some(120000));

    let report = paths.configured(&held).await.expect("doctor");
    assert!(report.ok, "expected healthy: {report:?}");
    let provider = report.provider.expect("provider report");
    assert_eq!(provider.profile.as_deref(), Some("openai"));
    assert_eq!(provider.kind.as_deref(), Some("openai_responses"));
    assert_eq!(provider.model.as_deref(), Some("gpt-5.5"));
    assert_eq!(provider.bytes, Some(120000));
    assert_eq!(provider.source.as_deref(), Some("provider_config"));
}

#[tokio::test]
async fn rejects() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    let store = super::support::bootstrap(&paths.database).await;
    store
        .seed(santi_core::GENESIS, &santi_core::now())
        .await
        .expect("seed");
    let held = runtime_under(temp.path(), None);

    let report = paths.configured(&held).await.expect("doctor");
    assert!(!report.ok);
    let provider = report.provider.expect("provider report");
    assert!(!provider.ok);
    assert_eq!(provider.profile.as_deref(), Some("openai"));
    assert_eq!(
        provider.error.as_deref(),
        Some("provider openai field bytes is required")
    );
}

fn paths_under(root: &Path) -> Layout {
    Layout {
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
        port: 0,
        provider: "openai".to_string(),
        providers: file.providers,
        environment: Default::default(),
        paths: paths_under(root),
        grace: Duration::from_secs(600),
        retention: Duration::from_secs(santi_api::RETENTION),
        github: Default::default(),
        feishu: Default::default(),
        capability: None,
        constitution: None,
    }
}
