use axum::http::HeaderMap;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;

use super::{
    NormalizedEvent, WebhookAdaptor, WebhookError, WebhookOutcome, sender_allowed, string_at,
};

type HmacSha256 = Hmac<Sha256>;

pub struct GithubAdaptor {
    self_login: Option<String>,
    allow: Option<String>,
}

impl GithubAdaptor {
    pub fn configured(self_login: Option<&str>, allow: Option<&str>) -> Self {
        Self {
            self_login: normalized(self_login),
            allow: normalized(allow),
        }
    }
}

impl WebhookAdaptor for GithubAdaptor {
    fn verify(
        &self,
        headers: &HeaderMap,
        raw_body: &[u8],
        secret: &str,
    ) -> Result<(), WebhookError> {
        let presented = headers
            .get("X-Hub-Signature-256")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("sha256="))
            .ok_or_else(|| {
                WebhookError::Unauthorized("missing X-Hub-Signature-256 header".to_string())
            })?;
        let presented = hex::decode(presented)
            .map_err(|_| WebhookError::Unauthorized("malformed signature hex".to_string()))?;
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
            .map_err(|error| WebhookError::Unauthorized(error.to_string()))?;
        mac.update(raw_body);
        mac.verify_slice(&presented)
            .map_err(|_| WebhookError::Unauthorized("signature mismatch".to_string()))
    }

    fn normalize(
        &self,
        headers: &HeaderMap,
        raw_body: &[u8],
        _secret: &str,
        webhook_name: &str,
    ) -> Result<WebhookOutcome, WebhookError> {
        let event_type = headers
            .get("X-GitHub-Event")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        let payload: Value = serde_json::from_slice(raw_body)
            .map_err(|error| WebhookError::BadRequest(format!("invalid JSON body: {error}")))?;

        if event_type != "issues" && event_type != "issue_comment" {
            return Ok(WebhookOutcome::Event(NormalizedEvent {
                santi_system_text: String::new(),
                label: format!("github:{webhook_name}:{event_type}"),
                metadata: None,
                delivery: None,
                in_scope: false,
                self_authored: false,
            }));
        }

        let repo = string_at(&payload, &["repository", "full_name"]).unwrap_or_default();
        let number = payload
            .pointer("/issue/number")
            .and_then(Value::as_i64)
            .unwrap_or_default();
        let action = payload
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let url = if event_type == "issue_comment" {
            string_at(&payload, &["comment", "html_url"])
        } else {
            string_at(&payload, &["issue", "html_url"])
        }
        .unwrap_or_default();
        let delivery = headers
            .get("X-GitHub-Delivery")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        let replay = (!delivery.is_empty()).then(|| delivery.clone());
        let sender = string_at(&payload, &["sender", "login"]).unwrap_or_default();
        let self_authored = self
            .self_login
            .as_ref()
            .is_some_and(|login| login.eq_ignore_ascii_case(&sender));
        let in_scope = sender_allowed(&sender, self.allow.as_deref());
        let santi_system_text = format!(
            "[github] {event_type}.{action} on {repo}#{number}\nurl: {url}\ndelivery: {delivery}"
        );
        let label = format!("github:{webhook_name}:issue:{repo}#{number}");
        let metadata = json!({
            "adaptor": "github",
            "webhook_name": webhook_name,
            "event_type": event_type,
            "action": action,
            "delivery": delivery,
            "repo": repo,
            "issue_number": number,
            "url": url,
            "label": label,
        });

        Ok(WebhookOutcome::Event(NormalizedEvent {
            santi_system_text,
            label,
            metadata: Some(metadata),
            delivery: replay,
            in_scope,
            self_authored,
        }))
    }
}

fn normalized(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}
