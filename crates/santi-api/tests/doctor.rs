use std::path::Path;

use santi_api::{
    config::{ConfigService, RuntimePaths},
    ops::doctor_configured_at,
};

#[test]
fn reports_budget() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    santi_core::SantiStore::open(&paths.database_path).expect("open store");
    let config = config_under(temp.path(), Some(120000));

    let report = doctor_configured_at(&paths, &config).expect("doctor");
    assert!(report.ok, "expected healthy: {report:?}");
    let provider = report.provider.expect("provider report");
    assert_eq!(provider.profile.as_deref(), Some("openai"));
    assert_eq!(provider.kind.as_deref(), Some("openai_responses"));
    assert_eq!(provider.model.as_deref(), Some("gpt-5.5"));
    assert_eq!(provider.input_budget_bytes, Some(120000));
    assert_eq!(provider.budget_source.as_deref(), Some("provider_config"));
}

#[test]
fn rejects_missing_budget() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    santi_core::SantiStore::open(&paths.database_path).expect("open store");
    let config = config_under(temp.path(), None);

    let report = doctor_configured_at(&paths, &config).expect("doctor");
    assert!(!report.ok);
    let provider = report.provider.expect("provider report");
    assert!(!provider.ok);
    assert_eq!(provider.profile.as_deref(), Some("openai"));
    assert_eq!(
        provider.error.as_deref(),
        Some("provider openai field input_budget_bytes is required")
    );
}

fn paths_under(root: &Path) -> RuntimePaths {
    RuntimePaths {
        database_path: root.join("runtime").join("db"),
        runtime_root: root.join("runtime"),
        execution_root: root.join("execution"),
    }
}

fn config_under(root: &Path, budget: Option<usize>) -> ConfigService {
    let budget = budget
        .map(|value| format!("input_budget_bytes = {value}"))
        .unwrap_or_default();
    let config_path = root.join("santi.toml");
    std::fs::write(
        &config_path,
        format!(
            r#"
            provider = "openai"

            [providers.openai]
            kind = "openai_responses"
            api_key = "test-key"
            model = "gpt-5.5"
            {budget}
            "#
        ),
    )
    .expect("write config");
    ConfigService::from_args([
        "santi".to_string(),
        "serve".to_string(),
        "--config".to_string(),
        config_path.display().to_string(),
    ])
    .expect("config service")
}
