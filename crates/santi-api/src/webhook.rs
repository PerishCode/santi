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
use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

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

/// What a verified request amounts to. Most requests normalize into an `Event`;
/// some integrations have control-plane requests that must be answered with a
/// JSON body on the spot (e.g. feishu's `url_verification` challenge echo) —
/// those short-circuit as `Reply` and never touch a strand.
#[derive(Debug, Clone)]
pub(crate) enum WebhookOutcome {
    Event(NormalizedEvent),
    Reply(Value),
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
    /// Verify the request's transport-level authenticity (signatures over the
    /// raw body). Fail-closed where the integration signs; integrations whose
    /// auth field travels INSIDE the (possibly encrypted) payload enforce it in
    /// `normalize` instead — between the two, an unauthenticated request never
    /// reaches a strand.
    fn verify(
        &self,
        headers: &HeaderMap,
        raw_body: &[u8],
        secret: &str,
    ) -> Result<(), WebhookError>;

    /// Open + normalize the raw event into santi's generic shape (or a direct
    /// `Reply` for control-plane requests). `secret` is passed through for
    /// integrations that authenticate inside the payload. `webhook_name` is the
    /// subscription name, woven into the label so distinct subscriptions never
    /// collide on a shared external id.
    fn normalize(
        &self,
        headers: &HeaderMap,
        raw_body: &[u8],
        secret: &str,
        webhook_name: &str,
    ) -> Result<WebhookOutcome, WebhookError>;
}

/// Map a subscription's `adaptor` string to its implementation.
pub(crate) fn adaptor_for(adaptor: &str) -> Option<Box<dyn WebhookAdaptor>> {
    match adaptor {
        "github" => Some(Box::new(GithubAdaptor)),
        "feishu" => Some(Box::new(FeishuAdaptor)),
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

        // Only issue threads are in scope for the first version. Anything else
        // (ping, push, …) verifies but produces no turn.
        if event_type != "issues" && event_type != "issue_comment" {
            return Ok(WebhookOutcome::Event(NormalizedEvent {
                santi_system_text: String::new(),
                label: format!("github:{webhook_name}:{event_type}"),
                in_scope: false,
                self_authored: false,
            }));
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

        Ok(WebhookOutcome::Event(NormalizedEvent {
            santi_system_text,
            label,
            in_scope,
            self_authored,
        }))
    }
}

/// Feishu (Lark) webhook adaptor — event subscription callbacks (schema 2.0).
///
/// Auth is two-layer, fail-closed overall: every payload carries the app's
/// Verification Token (v2 events in `header.token`, `url_verification` at top
/// level) which must equal the subscription secret — that check lives in
/// `normalize`, since the token travels INSIDE the possibly-encrypted payload.
/// When the app's Encrypt Key is configured (recommended), the payload arrives
/// AES-256-CBC encrypted and the request is signed (`X-Lark-Signature` =
/// hex(sha256(timestamp + nonce + encrypt_key + raw_body))) — `verify` enforces
/// the signature whenever it is presented.
///
/// `url_verification` (feishu probes the callback URL when it is configured) is
/// answered on the spot with the challenge echo (`WebhookOutcome::Reply`).
/// In-scope events: `im.message.receive_v1` from human senders — normalized into
/// a doorbell (chat + message locators, NO message content; the soul reads the
/// message itself through its carrier). One strand per feishu chat.
struct FeishuAdaptor;

/// Env var holding the app's Encrypt Key (payload decryption + request
/// signature). Optional but recommended; without it feishu sends plaintext.
const FEISHU_ENCRYPT_KEY_ENV: &str = "SANTI_WEBHOOK_FEISHU_ENCRYPT_KEY";

/// Env var holding a comma-separated allowlist of sender ids (open_id and/or
/// user_id) that may trigger a turn. Same policy shape as the github allowlist.
const FEISHU_ALLOW_ENV: &str = "SANTI_WEBHOOK_FEISHU_ALLOW";

impl WebhookAdaptor for FeishuAdaptor {
    fn verify(
        &self,
        headers: &HeaderMap,
        raw_body: &[u8],
        _secret: &str,
    ) -> Result<(), WebhookError> {
        feishu_verify_signature(
            headers,
            raw_body,
            feishu_env(FEISHU_ENCRYPT_KEY_ENV).as_deref(),
        )
    }

    fn normalize(
        &self,
        _headers: &HeaderMap,
        raw_body: &[u8],
        secret: &str,
        webhook_name: &str,
    ) -> Result<WebhookOutcome, WebhookError> {
        feishu_normalize(
            raw_body,
            secret,
            feishu_env(FEISHU_ENCRYPT_KEY_ENV).as_deref(),
            feishu_env(FEISHU_ALLOW_ENV).as_deref(),
            webhook_name,
        )
    }
}

/// Read a feishu policy env var, treating blank as unset.
fn feishu_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Enforce `X-Lark-Signature` when presented: hex(sha256(timestamp + nonce +
/// encrypt_key + raw_body)). A signed request without a configured key is
/// refused; an unsigned request passes here and is authenticated by the token
/// check in `feishu_normalize` (the token is inside the payload).
fn feishu_verify_signature(
    headers: &HeaderMap,
    raw_body: &[u8],
    encrypt_key: Option<&str>,
) -> Result<(), WebhookError> {
    let presented = headers
        .get("X-Lark-Signature")
        .and_then(|value| value.to_str().ok());
    let Some(presented) = presented else {
        return Ok(());
    };
    let key = encrypt_key.ok_or_else(|| {
        WebhookError::Unauthorized(format!(
            "signed payload but {FEISHU_ENCRYPT_KEY_ENV} is not set"
        ))
    })?;
    let timestamp = headers
        .get("X-Lark-Request-Timestamp")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            WebhookError::Unauthorized("missing X-Lark-Request-Timestamp header".to_string())
        })?;
    let nonce = headers
        .get("X-Lark-Request-Nonce")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            WebhookError::Unauthorized("missing X-Lark-Request-Nonce header".to_string())
        })?;
    let mut hasher = Sha256::new();
    hasher.update(timestamp.as_bytes());
    hasher.update(nonce.as_bytes());
    hasher.update(key.as_bytes());
    hasher.update(raw_body);
    let expected = hex::encode(hasher.finalize());
    if expected == presented.trim() {
        Ok(())
    } else {
        Err(WebhookError::Unauthorized(
            "feishu signature mismatch".to_string(),
        ))
    }
}

/// Open (decrypt if needed) + authenticate (token) + normalize a feishu payload.
fn feishu_normalize(
    raw_body: &[u8],
    secret: &str,
    encrypt_key: Option<&str>,
    allow: Option<&str>,
    webhook_name: &str,
) -> Result<WebhookOutcome, WebhookError> {
    let raw: Value = serde_json::from_slice(raw_body)
        .map_err(|error| WebhookError::BadRequest(format!("invalid JSON body: {error}")))?;

    // Encrypted payloads arrive as {"encrypt": "<base64>"} — open them first.
    let payload = match raw.get("encrypt").and_then(Value::as_str) {
        Some(ciphertext) => {
            let key = encrypt_key.ok_or_else(|| {
                WebhookError::Unauthorized(format!(
                    "encrypted payload but {FEISHU_ENCRYPT_KEY_ENV} is not set"
                ))
            })?;
            let plaintext = feishu_decrypt(key, ciphertext)?;
            serde_json::from_slice::<Value>(&plaintext).map_err(|error| {
                WebhookError::BadRequest(format!("invalid decrypted JSON: {error}"))
            })?
        }
        None => raw,
    };

    // Fail-closed token check — feishu's always-present auth field (top level on
    // url_verification, `header.token` on v2 events). Empty secret never passes
    // (the route already refuses a blank secret env, this is the backstop).
    let token = payload
        .get("token")
        .and_then(Value::as_str)
        .or_else(|| payload.pointer("/header/token").and_then(Value::as_str))
        .unwrap_or("");
    if secret.is_empty() || token != secret {
        return Err(WebhookError::Unauthorized(
            "feishu verification token mismatch".to_string(),
        ));
    }

    // Control-plane: answer the URL-configuration challenge on the spot.
    if payload.get("type").and_then(Value::as_str) == Some("url_verification") {
        let challenge = payload
            .get("challenge")
            .and_then(Value::as_str)
            .unwrap_or("");
        return Ok(WebhookOutcome::Reply(json!({ "challenge": challenge })));
    }

    // Only direct message receipt is in scope for now. Anything else verifies
    // but produces no turn.
    let event_type = payload
        .pointer("/header/event_type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if event_type != "im.message.receive_v1" {
        return Ok(WebhookOutcome::Event(NormalizedEvent {
            santi_system_text: String::new(),
            label: format!("feishu:{webhook_name}:{event_type}"),
            in_scope: false,
            self_authored: false,
        }));
    }

    // A doorbell, not a delivery: chat + message LOCATORS only. The event body
    // carries the message content — deliberately dropped; the soul reads the
    // message itself through its feishu carrier.
    let chat_id = string_at(&payload, &["event", "message", "chat_id"]).unwrap_or_default();
    let chat_type = string_at(&payload, &["event", "message", "chat_type"]).unwrap_or_default();
    let message_id = string_at(&payload, &["event", "message", "message_id"]).unwrap_or_default();
    let event_id = string_at(&payload, &["header", "event_id"]).unwrap_or_default();

    // Sender gate: only human senders, and only allowlisted ones when the
    // allowlist is set (either open_id or user_id may match). Bots (including
    // this app) come through with sender_type != "user" and stay out of scope.
    let sender_type = string_at(&payload, &["event", "sender", "sender_type"]).unwrap_or_default();
    let open_id =
        string_at(&payload, &["event", "sender", "sender_id", "open_id"]).unwrap_or_default();
    let user_id =
        string_at(&payload, &["event", "sender", "sender_id", "user_id"]).unwrap_or_default();
    let allowed = sender_allowed(&open_id, allow) || sender_allowed(&user_id, allow);
    let in_scope = sender_type == "user" && allowed;

    // The doorbell: occurrence kind + address. The soul goes and looks.
    let santi_system_text = format!(
        "[feishu] im.message.receive_v1 in chat {chat_id} ({chat_type})\nmessage_id: {message_id}\nevent_id: {event_id}"
    );
    // One strand per feishu chat, scoped by subscription name.
    let label = format!("feishu:{webhook_name}:chat:{chat_id}");

    Ok(WebhookOutcome::Event(NormalizedEvent {
        santi_system_text,
        label,
        in_scope,
        self_authored: false,
    }))
}

/// Decrypt feishu's `encrypt` field: base64( iv[16] || AES-256-CBC(payload) )
/// with key = sha256(encrypt_key), PKCS7 padding.
fn feishu_decrypt(encrypt_key: &str, ciphertext_b64: &str) -> Result<Vec<u8>, WebhookError> {
    use aes::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
    let data = base64::engine::general_purpose::STANDARD
        .decode(ciphertext_b64)
        .map_err(|_| WebhookError::BadRequest("malformed encrypt field base64".to_string()))?;
    if data.len() <= 16 {
        return Err(WebhookError::BadRequest(
            "encrypted payload too short".to_string(),
        ));
    }
    let key = Sha256::digest(encrypt_key.as_bytes());
    let (iv, ciphertext) = data.split_at(16);
    cbc::Decryptor::<aes::Aes256>::new_from_slices(&key, iv)
        .map_err(|error| WebhookError::Unauthorized(error.to_string()))?
        .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
        .map_err(|_| {
            WebhookError::Unauthorized("feishu decryption failed (wrong encrypt key?)".to_string())
        })
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

    fn expect_event(outcome: WebhookOutcome) -> NormalizedEvent {
        match outcome {
            WebhookOutcome::Event(event) => event,
            WebhookOutcome::Reply(value) => panic!("expected event, got reply {value}"),
        }
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
        let event = expect_event(
            GithubAdaptor
                .normalize(
                    &headers("issue_comment", None),
                    issue_comment_body(),
                    SECRET,
                    "ops",
                )
                .expect("normalize"),
        );
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
        let event = expect_event(
            GithubAdaptor
                .normalize(&headers("ping", None), body, SECRET, "ops")
                .expect("normalize"),
        );
        assert!(!event.in_scope);
    }

    // ---- feishu ----

    const FEISHU_TOKEN: &str = "verification-token-1";
    const FEISHU_KEY: &str = "encrypt-key-1";

    /// Test-side inverse of `feishu_decrypt`: base64( iv || AES-256-CBC(plain) ).
    fn feishu_encrypt(encrypt_key: &str, plaintext: &[u8]) -> String {
        use aes::cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
        let key = Sha256::digest(encrypt_key.as_bytes());
        let iv = [7u8; 16];
        let ciphertext = cbc::Encryptor::<aes::Aes256>::new_from_slices(&key, &iv)
            .unwrap()
            .encrypt_padded_vec_mut::<Pkcs7>(plaintext);
        let mut data = iv.to_vec();
        data.extend_from_slice(&ciphertext);
        base64::engine::general_purpose::STANDARD.encode(data)
    }

    fn feishu_message_payload(sender_type: &str, open_id: &str) -> String {
        format!(
            r#"{{
                "schema": "2.0",
                "header": {{
                    "event_id": "ev_123",
                    "event_type": "im.message.receive_v1",
                    "token": "{FEISHU_TOKEN}",
                    "app_id": "cli_x"
                }},
                "event": {{
                    "sender": {{
                        "sender_type": "{sender_type}",
                        "sender_id": {{ "open_id": "{open_id}", "user_id": "u_1" }}
                    }},
                    "message": {{
                        "chat_id": "oc_chat1",
                        "chat_type": "p2p",
                        "message_id": "om_msg1",
                        "message_type": "text",
                        "content": "{{\"text\":\"hello liberte, top secret\"}}"
                    }}
                }}
            }}"#
        )
    }

    #[test]
    fn feishu_answers_url_verification_challenge() {
        let body = format!(
            r#"{{ "challenge": "ch-42", "token": "{FEISHU_TOKEN}", "type": "url_verification" }}"#
        );
        let outcome =
            feishu_normalize(body.as_bytes(), FEISHU_TOKEN, None, None, "chat").expect("normalize");
        match outcome {
            WebhookOutcome::Reply(value) => {
                assert_eq!(value, json!({ "challenge": "ch-42" }));
            }
            WebhookOutcome::Event(_) => panic!("expected challenge reply"),
        }
    }

    #[test]
    fn feishu_rejects_bad_token() {
        let body = r#"{ "challenge": "ch", "token": "someone-else", "type": "url_verification" }"#;
        let result = feishu_normalize(body.as_bytes(), FEISHU_TOKEN, None, None, "chat");
        assert!(matches!(result, Err(WebhookError::Unauthorized(_))));
        // An absent token is refused the same way (fail-closed).
        let result = feishu_normalize(
            br#"{ "type": "url_verification" }"#,
            FEISHU_TOKEN,
            None,
            None,
            "chat",
        );
        assert!(matches!(result, Err(WebhookError::Unauthorized(_))));
    }

    #[test]
    fn feishu_decrypts_and_normalizes_message_to_a_doorbell() {
        let plain = feishu_message_payload("user", "ou_alice");
        let body = format!(
            r#"{{ "encrypt": "{}" }}"#,
            feishu_encrypt(FEISHU_KEY, plain.as_bytes())
        );
        let event = expect_event(
            feishu_normalize(
                body.as_bytes(),
                FEISHU_TOKEN,
                Some(FEISHU_KEY),
                Some("ou_alice"),
                "chat",
            )
            .expect("normalize"),
        );
        assert!(event.in_scope);
        assert!(!event.self_authored);
        assert_eq!(event.label, "feishu:chat:chat:oc_chat1");
        // Doorbell: occurrence kind + address (locators only).
        assert!(
            event
                .santi_system_text
                .contains("[feishu] im.message.receive_v1 in chat oc_chat1 (p2p)")
        );
        assert!(event.santi_system_text.contains("message_id: om_msg1"));
        // NOT a delivery: the message content is never pushed.
        assert!(!event.santi_system_text.contains("top secret"));
        // And the sender ids are locator-gates, not content.
        assert!(!event.santi_system_text.contains("ou_alice"));
    }

    #[test]
    fn feishu_refuses_encrypted_payload_without_key() {
        let plain = feishu_message_payload("user", "ou_alice");
        let body = format!(
            r#"{{ "encrypt": "{}" }}"#,
            feishu_encrypt(FEISHU_KEY, plain.as_bytes())
        );
        let result = feishu_normalize(body.as_bytes(), FEISHU_TOKEN, None, None, "chat");
        assert!(matches!(result, Err(WebhookError::Unauthorized(_))));
        // Wrong key -> decryption failure, refused.
        let result = feishu_normalize(
            body.as_bytes(),
            FEISHU_TOKEN,
            Some("wrong-key"),
            None,
            "chat",
        );
        assert!(matches!(result, Err(WebhookError::Unauthorized(_))));
    }

    #[test]
    fn feishu_gates_senders() {
        // Non-user senders (bots, incl. this app's own echoes) stay out of scope.
        let plain = feishu_message_payload("app", "ou_botself");
        let event = expect_event(
            feishu_normalize(plain.as_bytes(), FEISHU_TOKEN, None, None, "chat")
                .expect("normalize"),
        );
        assert!(!event.in_scope);
        // Allowlist set -> out-of-list human is a 200-noop.
        let plain = feishu_message_payload("user", "ou_stranger");
        let event = expect_event(
            feishu_normalize(
                plain.as_bytes(),
                FEISHU_TOKEN,
                None,
                Some("ou_operator"),
                "chat",
            )
            .expect("normalize"),
        );
        assert!(!event.in_scope);
        // user_id may match the allowlist instead of open_id.
        let plain = feishu_message_payload("user", "ou_whoever");
        let event = expect_event(
            feishu_normalize(plain.as_bytes(), FEISHU_TOKEN, None, Some("u_1"), "chat")
                .expect("normalize"),
        );
        assert!(event.in_scope);
    }

    #[test]
    fn feishu_ignores_out_of_scope_event_type() {
        let body = format!(
            r#"{{
                "schema": "2.0",
                "header": {{ "event_type": "im.chat.updated_v1", "token": "{FEISHU_TOKEN}" }},
                "event": {{}}
            }}"#
        );
        let event = expect_event(
            feishu_normalize(body.as_bytes(), FEISHU_TOKEN, None, None, "chat").expect("normalize"),
        );
        assert!(!event.in_scope);
    }

    #[test]
    fn feishu_signature_verification() {
        let body = b"{}";
        let timestamp = "1700000000";
        let nonce = "nonce-1";
        let mut hasher = Sha256::new();
        hasher.update(timestamp.as_bytes());
        hasher.update(nonce.as_bytes());
        hasher.update(FEISHU_KEY.as_bytes());
        hasher.update(body);
        let good = hex::encode(hasher.finalize());

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-lark-request-timestamp",
            HeaderValue::from_str(timestamp).unwrap(),
        );
        headers.insert(
            "x-lark-request-nonce",
            HeaderValue::from_str(nonce).unwrap(),
        );
        headers.insert("x-lark-signature", HeaderValue::from_str(&good).unwrap());

        // Good signature with the right key.
        assert!(feishu_verify_signature(&headers, body, Some(FEISHU_KEY)).is_ok());
        // Signed request without a configured key is refused.
        assert!(matches!(
            feishu_verify_signature(&headers, body, None),
            Err(WebhookError::Unauthorized(_))
        ));
        // Tampered body -> mismatch.
        assert!(matches!(
            feishu_verify_signature(&headers, b"{ }", Some(FEISHU_KEY)),
            Err(WebhookError::Unauthorized(_))
        ));
        // Unsigned request passes this layer (token check authenticates it later).
        assert!(feishu_verify_signature(&HeaderMap::new(), body, Some(FEISHU_KEY)).is_ok());
    }
}
