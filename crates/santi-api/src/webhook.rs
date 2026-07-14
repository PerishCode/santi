use axum::http::HeaderMap;
use serde_json::Value;

pub mod feishu;
pub mod github;

use feishu::FeishuAdaptor;
use github::GithubAdaptor;

#[derive(Debug, Clone)]
pub struct NormalizedEvent {
    pub santi_system_text: String,
    pub label: String,
    pub source_metadata: Option<Value>,
    pub in_scope: bool,
    pub self_authored: bool,
}

#[derive(Debug, Clone)]
pub enum WebhookOutcome {
    Event(NormalizedEvent),
    Reply(Value),
}

#[derive(Debug)]
pub enum WebhookError {
    Unauthorized(String),
    BadRequest(String),
}

pub trait WebhookAdaptor: Send + Sync {
    fn verify(
        &self,
        headers: &HeaderMap,
        raw_body: &[u8],
        secret: &str,
    ) -> Result<(), WebhookError>;

    fn normalize(
        &self,
        headers: &HeaderMap,
        raw_body: &[u8],
        secret: &str,
        webhook_name: &str,
    ) -> Result<WebhookOutcome, WebhookError>;
}

pub(crate) fn adaptor_for(adaptor: &str) -> Option<Box<dyn WebhookAdaptor>> {
    match adaptor {
        "github" => Some(Box::new(GithubAdaptor::from_env())),
        "feishu" => Some(Box::new(FeishuAdaptor::from_env())),
        _ => None,
    }
}

fn sender_allowed(sender: &str, allow: Option<&str>) -> bool {
    match allow.map(str::trim) {
        None | Some("") => true,
        Some(list) => list
            .split(',')
            .any(|entry| entry.trim().eq_ignore_ascii_case(sender)),
    }
}

fn string_at(value: &Value, path: &[&str]) -> Option<String> {
    let mut cursor = value;
    for key in path {
        cursor = cursor.get(key)?;
    }
    cursor.as_str().map(str::to_string)
}
