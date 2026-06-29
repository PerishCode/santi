use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use santi_api::config::{AppCommand, ChatCompletionsConfig, ConfigService, ProviderConfig};

#[test]
fn resolves_chat_completions_profile() {
    let path = write_config(
        r#"
        provider = "openai"

        [providers.openai]
        kind = "openai_responses"
        api_key = "openai-key"
        model = "gpt-5.5"

        [providers.siliconflow]
        kind = "chat_completions"
        api_key = "test-key"
        model = "zai-org/GLM-5.2"
        base_url = "https://api.siliconflow.cn/v1"
        thinking = ""
        reasoning_effort = ""
        max_tokens = 2048
        "#,
    );

    let service = ConfigService::from_args(args([
        "santi-api",
        "serve",
        "--config",
        path.to_str().expect("config path"),
        "--provider=siliconflow",
    ]))
    .expect("config service");

    assert_eq!(service.command(), AppCommand::Serve);
    assert_eq!(
        service.provider_config().expect("provider config"),
        ProviderConfig::ChatCompletions(ChatCompletionsConfig {
            provider: "siliconflow".to_string(),
            api_key: "test-key".to_string(),
            model: "zai-org/GLM-5.2".to_string(),
            base_url: "https://api.siliconflow.cn/v1".to_string(),
            thinking: None,
            reasoning_effort: None,
            max_tokens: Some(2048),
        })
    );

    let _ = fs::remove_file(path);
}

#[test]
fn parses_export_openapi_command() {
    let service =
        ConfigService::from_args(args(["santi-api", "export-openapi"])).expect("config service");

    assert_eq!(service.command(), AppCommand::ExportOpenApi);
}

#[test]
fn reports_missing_field() {
    let path = write_config(
        r#"
        provider = "openai"

        [providers.openai]
        kind = "openai_responses"
        api_key = "openai-key"
        model = ""
        "#,
    );

    let service = ConfigService::from_args(args([
        "santi-api",
        "--config",
        path.to_str().expect("config path"),
    ]))
    .expect("config service");

    assert_eq!(
        service.provider_config().expect_err("missing model"),
        "provider openai field model is required"
    );

    let _ = fs::remove_file(path);
}

fn args<const N: usize>(values: [&str; N]) -> impl IntoIterator<Item = String> {
    values.map(str::to_string)
}

fn write_config(content: &str) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    // A per-call counter keeps parallel tests from colliding on the same name.
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("santi-config-{id}-{seq}.toml"));
    fs::write(&path, content).expect("write config");
    path
}
