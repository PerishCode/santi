//! Webhook adaptors — the boundary where integration knowledge lives.
//!
//! An adaptor is a DOORBELL, not a delivery: it verifies an external request's
//! authenticity against a shared secret, then normalizes it into the minimum
//! needed to (a) map it to a strand and (b) tell the soul an occurrence happened
//! at an address. It does NOT push world-content (issue body, comment list, state,
//! history) — the soul perceives that itself by looking through its carrier and
//! remembers it in its own memory, like a natural person who hears a knock and
//! then goes to see. Core stays unaware of any provider's payload; everything
//! integration-specific (signatures, event types, locators) is here.

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
    /// The doorbell text: occurrence kind + address (e.g. repo#N + url), NOT
    /// world-content. Enough for the soul to know what happened and where to look.
    pub santi_system_text: String,
    /// The opaque external label that anchors the strand (per-thread identity).
    pub label: String,
    /// Whether this event type is in scope for santi. Out-of-scope events (a
    /// GitHub `ping`, an unhandled action) verify fine but produce no turn.
    pub in_scope: bool,
    /// Whether the event is the box vessel's own echo — a vessel-level loop guard
    /// (NOT the soul's identity, which lives in memory). When true the route drops
    /// it without waking the soul.
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
/// raw body) and turns `issues` / `issue_comment` events into a doorbell (locator
/// plus url), pushing no issue/comment content. Events whose sender matches the
/// box vessel's configured GitHub login are dropped to break the act-then-observe
/// loop; with no login configured the backstop is the soul reading the thread and
/// choosing to do nothing. An optional sender allowlist narrows who may wake the
/// soul at all (a safety rail on a public repo, where any user can comment).
struct GithubAdaptor;

/// Env var naming the box vessel's GitHub login, for the self-echo loop guard.
const GITHUB_SELF_LOGIN_ENV: &str = "SANTI_WEBHOOK_GITHUB_LOGIN";

/// Env var holding a comma-separated allowlist of sender logins that may trigger a
/// turn. Policy-in-config, like `GITHUB_SELF_LOGIN_ENV`. Unset/empty = no allowlist
/// (any sender in scope); set = only listed senders wake the soul, the rest 200-noop.
const GITHUB_ALLOW_ENV: &str = "SANTI_WEBHOOK_GITHUB_ALLOW";

/// Whether `sender` may trigger a turn given the configured allowlist. A None or
/// blank allowlist imposes no restriction; otherwise the sender must match one
/// entry (case-insensitive, surrounding whitespace ignored).
fn sender_allowed(sender: &str, allow: Option<&str>) -> bool {
    match allow.map(str::trim) {
        None | Some("") => true,
        Some(list) => list
            .split(',')
            .any(|entry| entry.trim().eq_ignore_ascii_case(sender)),
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

        // A doorbell, not a delivery: extract only enough to LOCATE the
        // occurrence (repo + issue number → the address) and name what happened.
        // The issue's title, body, comment list, current state and history are
        // NOT pushed — the soul reads them itself from the world (its carrier)
        // and remembers in its own memory. So we keep no `title`/`body` here.
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

        // Self-authored loop guard: `sender.login` is read ONLY to drop the
        // box vessel's own echoes — it is not pushed as content. It is not the
        // soul's identity (that lives in memory); just a vessel-level echo drop.
        let sender = string_at(&payload, &["sender", "login"]).unwrap_or_default();
        let self_authored = env::var(GITHUB_SELF_LOGIN_ENV)
            .ok()
            .map(|login| login.trim().to_string())
            .filter(|login| !login.is_empty())
            .is_some_and(|login| login.eq_ignore_ascii_case(&sender));

        // Sender allowlist: an extra gate on who may wake the soul. With the
        // allowlist set, an out-of-list sender is in_scope=false (200 noop).
        let in_scope = sender_allowed(&sender, env::var(GITHUB_ALLOW_ENV).ok().as_deref());

        // The doorbell: occurrence kind + address. The soul goes and looks.
        let santi_system_text = format!(
            "[github] {event_type}.{action} on {repo}#{number}\nurl: {url}\ndelivery: {delivery}"
        );
        // One strand per issue thread, scoped by subscription name so two
        // subscriptions never share a thread.
        let label = format!("github:{webhook_name}:issue:{repo}#{number}");

        Ok(NormalizedEvent {
            santi_system_text,
            label,
            in_scope,
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
    fn normalizes_issue_comment_to_a_doorbell() {
        let event = GithubAdaptor
            .normalize(&headers("issue_comment", None), issue_comment_body(), "ops")
            .expect("normalize");
        assert!(event.in_scope);
        assert!(!event.self_authored);
        assert_eq!(event.label, "github:ops:issue:PerishCode/santi#42");
        // Doorbell: occurrence kind + address.
        assert!(
            event
                .santi_system_text
                .contains("[github] issue_comment.created on PerishCode/santi#42")
        );
        assert!(
            event
                .santi_system_text
                .contains("https://github.com/PerishCode/santi/issues/42#issuecomment-1")
        );
        // NOT a delivery: world-content is never pushed — the soul reads it itself.
        assert!(
            !event
                .santi_system_text
                .contains("hey santi, can you take a look?")
        );
        assert!(!event.santi_system_text.contains("Give santi senses"));
    }

    #[test]
    fn sender_allowlist_gates_triggers() {
        // No allowlist configured -> anyone is allowed.
        assert!(sender_allowed("anyone", None));
        assert!(sender_allowed("anyone", Some("")));
        assert!(sender_allowed("anyone", Some("   ")));
        // Allowlist set -> only listed senders, case-insensitive, whitespace-trimmed.
        assert!(sender_allowed("PerishCode", Some("PerishCode")));
        assert!(sender_allowed("perishcode", Some("PerishCode")));
        assert!(sender_allowed("bob", Some(" alice , bob , PerishCode ")));
        assert!(!sender_allowed("stranger", Some("PerishCode")));
        assert!(!sender_allowed("LiberteCode", Some("PerishCode")));
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
