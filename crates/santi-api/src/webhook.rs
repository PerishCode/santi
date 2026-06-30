//! Webhook adaptors — the boundary where integration knowledge lives.
//!
//! An adaptor verifies an external request's authenticity against a shared secret
//! and normalizes it into santi's generic shape (an opaque `santi-system` text + a
//! session label). Core stays unaware of any provider's payload; everything
//! integration-specific (signatures, event types, self-identity, labels) is here.

use std::env;

use axum::http::HeaderMap;
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// An external event normalized into santi's generic shape. The adaptor owns all
/// integration knowledge; core sees only the opaque `santi_system_text` + `label`.
#[derive(Debug, Clone)]
pub(crate) struct NormalizedEvent {
    /// The `santi-system` message text addressed to the soul (curated fields).
    pub santi_system_text: String,
    /// The opaque external label that anchors the session (per-thread identity).
    pub label: String,
    /// Whether this event type is in scope for santi. Out-of-scope events (a
    /// GitHub `ping`, an unhandled action) verify fine but produce no turn.
    pub in_scope: bool,
    /// Whether the event was authored by santi's own identity — the loop guard.
    /// When true the route drops it without waking the soul.
    pub self_authored: bool,
}

/// Why an adaptor refused a request. Maps to the route's HTTP status.
#[derive(Debug)]
pub(crate) enum WebhookError {
    /// Signature missing or did not verify (also a missing secret). Fail-closed.
    Unauthorized(String),
    /// Body could not be parsed into the expected shape.
    BadRequest(String),
}

/// The boundary normalizer for one external source. Mechanism is generic; each
/// impl carries the policy/knowledge for its integration.
pub(crate) trait WebhookAdaptor: Send + Sync {
    /// Verify the request is authentic against the shared `secret`. Fail-closed:
    /// a missing signature or secret is an error, never a pass.
    fn verify(
        &self,
        headers: &HeaderMap,
        raw_body: &[u8],
        secret: &str,
    ) -> Result<(), WebhookError>;

    /// Normalize the raw event into santi's generic shape. `webhook_name` is the
    /// subscription name, woven into the label so distinct subscriptions never
    /// collide on a shared external id.
    fn normalize(
        &self,
        headers: &HeaderMap,
        raw_body: &[u8],
        webhook_name: &str,
    ) -> Result<NormalizedEvent, WebhookError>;
}

/// Map a subscription's `adaptor` string to its implementation.
pub(crate) fn adaptor_for(adaptor: &str) -> Option<Box<dyn WebhookAdaptor>> {
    match adaptor {
        "github" => Some(Box::new(GithubAdaptor)),
        _ => None,
    }
}

/// GitHub webhook adaptor. Verifies `X-Hub-Signature-256` (HMAC-SHA256 over the
/// raw body) and normalizes `issues` / `issue_comment` events. Self-authored
/// events (those whose sender matches the box's configured GitHub login) are
/// dropped to break the act→observe→act loop; with no login configured the
/// backstop is the soul reading its own comment and choosing to do nothing.
struct GithubAdaptor;

/// Env var naming the box's own GitHub login, for the self-authored loop guard.
const GITHUB_SELF_LOGIN_ENV: &str = "SANTI_WEBHOOK_GITHUB_LOGIN";

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
        // `verify_slice` is constant-time.
        mac.verify_slice(&presented)
            .map_err(|_| WebhookError::Unauthorized("signature mismatch".to_string()))
    }

    fn normalize(
        &self,
        headers: &HeaderMap,
        raw_body: &[u8],
        webhook_name: &str,
    ) -> Result<NormalizedEvent, WebhookError> {
        let event_type = headers
            .get("X-GitHub-Event")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        let payload: Value = serde_json::from_slice(raw_body)
            .map_err(|error| WebhookError::BadRequest(format!("invalid JSON body: {error}")))?;

        // Only issue threads are in scope for the first version. Anything else
        // (ping, push, …) verifies but produces no turn.
        if event_type != "issues" && event_type != "issue_comment" {
            return Ok(NormalizedEvent {
                santi_system_text: String::new(),
                label: format!("github:{webhook_name}:{event_type}"),
                in_scope: false,
                self_authored: false,
            });
        }

        let repo = string_at(&payload, &["repository", "full_name"]).unwrap_or_default();
        let number = payload
            .pointer("/issue/number")
            .and_then(Value::as_i64)
            .unwrap_or_default();
        let title = string_at(&payload, &["issue", "title"]).unwrap_or_default();
        let action = payload
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let sender = string_at(&payload, &["sender", "login"]).unwrap_or_default();

        let self_authored = env::var(GITHUB_SELF_LOGIN_ENV)
            .ok()
            .map(|login| login.trim().to_string())
            .filter(|login| !login.is_empty())
            .is_some_and(|login| login.eq_ignore_ascii_case(&sender));

        let (body, url) = if event_type == "issue_comment" {
            (
                string_at(&payload, &["comment", "body"]).unwrap_or_default(),
                string_at(&payload, &["comment", "html_url"]).unwrap_or_default(),
            )
        } else {
            (
                string_at(&payload, &["issue", "body"]).unwrap_or_default(),
                string_at(&payload, &["issue", "html_url"]).unwrap_or_default(),
            )
        };

        let santi_system_text = format!(
            "[github] {event_type}.{action}\n\
             repo: {repo}\n\
             issue: #{number} {title:?}\n\
             url: {url}\n\
             author: {sender}\n\
             ---\n\
             {body}"
        );
        // One session per issue thread, scoped by subscription name so two
        // subscriptions never share a thread.
        let label = format!("github:{webhook_name}:issue:{repo}#{number}");

        Ok(NormalizedEvent {
            santi_system_text,
            label,
            in_scope: true,
            self_authored,
        })
    }
}

/// Read a nested string field (`pointer`-style path) from a JSON value.
fn string_at(value: &Value, path: &[&str]) -> Option<String> {
    let mut cursor = value;
    for key in path {
        cursor = cursor.get(key)?;
    }
    cursor.as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    const SECRET: &str = "it's-a-secret-to-everybody";

    fn sign(secret: &str, body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    fn headers(event: &str, signature: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-github-event", HeaderValue::from_str(event).unwrap());
        if let Some(signature) = signature {
            headers.insert(
                "x-hub-signature-256",
                HeaderValue::from_str(signature).unwrap(),
            );
        }
        headers
    }

    fn issue_comment_body() -> &'static [u8] {
        br#"{
            "action": "created",
            "repository": { "full_name": "PerishCode/santi" },
            "issue": { "number": 42, "title": "Give santi senses" },
            "comment": {
                "body": "hey santi, can you take a look?",
                "html_url": "https://github.com/PerishCode/santi/issues/42#issuecomment-1"
            },
            "sender": { "login": "somehuman" }
        }"#
    }

    #[test]
    fn verifies_good_signature() {
        let body = issue_comment_body();
        let signature = sign(SECRET, body);
        let result =
            GithubAdaptor.verify(&headers("issue_comment", Some(&signature)), body, SECRET);
        assert!(result.is_ok());
    }

    #[test]
    fn rejects_bad_signature() {
        let body = issue_comment_body();
        let signature = sign("wrong-secret", body);
        let result =
            GithubAdaptor.verify(&headers("issue_comment", Some(&signature)), body, SECRET);
        assert!(matches!(result, Err(WebhookError::Unauthorized(_))));
    }

    #[test]
    fn rejects_missing_signature() {
        let body = issue_comment_body();
        let result = GithubAdaptor.verify(&headers("issue_comment", None), body, SECRET);
        assert!(matches!(result, Err(WebhookError::Unauthorized(_))));
    }

    #[test]
    fn normalizes_issue_comment() {
        let event = GithubAdaptor
            .normalize(&headers("issue_comment", None), issue_comment_body(), "ops")
            .expect("normalize");
        assert!(event.in_scope);
        assert!(!event.self_authored);
        assert_eq!(event.label, "github:ops:issue:PerishCode/santi#42");
        assert!(
            event
                .santi_system_text
                .contains("[github] issue_comment.created")
        );
        assert!(event.santi_system_text.contains("repo: PerishCode/santi"));
        assert!(
            event
                .santi_system_text
                .contains("hey santi, can you take a look?")
        );
    }

    #[test]
    fn ignores_out_of_scope_event() {
        let body = br#"{ "zen": "Keep it logically awesome." }"#;
        let event = GithubAdaptor
            .normalize(&headers("ping", None), body, "ops")
            .expect("normalize");
        assert!(!event.in_scope);
    }
}
