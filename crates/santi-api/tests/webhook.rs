use aes::cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use axum::http::{HeaderMap, HeaderValue};
use base64::Engine as _;
use hmac::{Hmac, Mac};
use santi_api::webhook::{
    NormalizedEvent, WebhookAdaptor, WebhookError, WebhookOutcome, feishu::FeishuAdaptor,
    github::GithubAdaptor,
};
use serde_json::json;
use sha2::{Digest, Sha256};

const SECRET: &str = "test-secret";
const FEISHU_TOKEN: &str = "verification-token-1";
const FEISHU_KEY: &str = "encrypt-key-1";

#[test]
fn github_verifies_signature() {
    let body = github_body();
    let signature = sign(SECRET, body);
    let adaptor = GithubAdaptor::configured(None, None);
    assert!(
        adaptor
            .verify(
                &github_headers("issue_comment", Some(&signature)),
                body,
                SECRET
            )
            .is_ok()
    );
    assert!(matches!(
        adaptor.verify(
            &github_headers("issue_comment", Some(&sign("wrong", body))),
            body,
            SECRET
        ),
        Err(WebhookError::Unauthorized(_))
    ));
    assert!(matches!(
        adaptor.verify(&github_headers("issue_comment", None), body, SECRET),
        Err(WebhookError::Unauthorized(_))
    ));
}

#[test]
fn github_normalizes_event() {
    let event = expect_event(
        GithubAdaptor::configured(None, None)
            .normalize(
                &github_headers("issue_comment", None),
                github_body(),
                SECRET,
                "ops",
            )
            .expect("normalize"),
    );
    assert!(event.in_scope);
    assert_eq!(event.label, "github:ops:issue:PerishCode/santi#42");
    assert!(event.santi_system_text.contains("issue_comment.created"));
    assert!(event.santi_system_text.contains("issuecomment-1"));
    assert!(!event.santi_system_text.contains("top secret"));
}

#[test]
fn github_gates_senders() {
    let headers = github_headers("issue_comment", None);
    let allowed = expect_event(
        GithubAdaptor::configured(None, Some("SomeHuman"))
            .normalize(&headers, github_body(), SECRET, "ops")
            .expect("normalize"),
    );
    assert!(allowed.in_scope);
    let rejected = expect_event(
        GithubAdaptor::configured(None, Some("someone-else"))
            .normalize(&headers, github_body(), SECRET, "ops")
            .expect("normalize"),
    );
    assert!(!rejected.in_scope);
    let own = expect_event(
        GithubAdaptor::configured(Some("somehuman"), None)
            .normalize(&headers, github_body(), SECRET, "ops")
            .expect("normalize"),
    );
    assert!(own.self_authored);
}

#[test]
fn github_ignores_ping() {
    let event = expect_event(
        GithubAdaptor::configured(None, None)
            .normalize(
                &github_headers("ping", None),
                br#"{"zen":"ok"}"#,
                SECRET,
                "ops",
            )
            .expect("normalize"),
    );
    assert!(!event.in_scope);
}

#[test]
fn feishu_answers_challenge() {
    let body =
        format!(r#"{{"challenge":"ch-42","token":"{FEISHU_TOKEN}","type":"url_verification"}}"#);
    let outcome = FeishuAdaptor::configured(None, None)
        .normalize(&HeaderMap::new(), body.as_bytes(), FEISHU_TOKEN, "chat")
        .expect("normalize");
    assert!(
        matches!(outcome, WebhookOutcome::Reply(value) if value == json!({"challenge":"ch-42"}))
    );
}

#[test]
fn feishu_rejects_token() {
    let body = br#"{"challenge":"ch","token":"wrong","type":"url_verification"}"#;
    let result = FeishuAdaptor::configured(None, None).normalize(
        &HeaderMap::new(),
        body,
        FEISHU_TOKEN,
        "chat",
    );
    assert!(matches!(result, Err(WebhookError::Unauthorized(_))));
}

#[test]
fn feishu_normalizes_encrypted() {
    let plain = feishu_payload("user", "ou_alice", "im.message.receive_v1");
    let body = format!(r#"{{"encrypt":"{}"}}"#, encrypt(&plain));
    let event = expect_event(
        FeishuAdaptor::configured(Some(FEISHU_KEY), Some("ou_alice"))
            .normalize(&HeaderMap::new(), body.as_bytes(), FEISHU_TOKEN, "chat")
            .expect("normalize"),
    );
    assert!(event.in_scope);
    assert_eq!(event.label, "feishu:chat:chat:oc_chat1");
    assert!(event.santi_system_text.contains("message_id: om_msg1"));
    assert!(!event.santi_system_text.contains("top secret"));

    let result = FeishuAdaptor::configured(None, None).normalize(
        &HeaderMap::new(),
        body.as_bytes(),
        FEISHU_TOKEN,
        "chat",
    );
    assert!(matches!(result, Err(WebhookError::Unauthorized(_))));
}

#[test]
fn feishu_gates_senders() {
    let bot = feishu_payload("app", "ou_bot", "im.message.receive_v1");
    let event = expect_event(
        FeishuAdaptor::configured(None, None)
            .normalize(&HeaderMap::new(), bot.as_bytes(), FEISHU_TOKEN, "chat")
            .expect("normalize"),
    );
    assert!(!event.in_scope);

    let human = feishu_payload("user", "ou_stranger", "im.message.receive_v1");
    let event = expect_event(
        FeishuAdaptor::configured(None, Some("ou_operator"))
            .normalize(&HeaderMap::new(), human.as_bytes(), FEISHU_TOKEN, "chat")
            .expect("normalize"),
    );
    assert!(!event.in_scope);
}

#[test]
fn feishu_verifies_signature() {
    let body = b"{}";
    let timestamp = "1700000000";
    let nonce = "nonce-1";
    let mut hasher = Sha256::new();
    hasher.update(timestamp.as_bytes());
    hasher.update(nonce.as_bytes());
    hasher.update(FEISHU_KEY.as_bytes());
    hasher.update(body);
    let signature = hex::encode(hasher.finalize());
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-lark-request-timestamp",
        HeaderValue::from_static(timestamp),
    );
    headers.insert("x-lark-request-nonce", HeaderValue::from_static(nonce));
    headers.insert(
        "x-lark-signature",
        HeaderValue::from_str(&signature).unwrap(),
    );
    assert!(
        FeishuAdaptor::configured(Some(FEISHU_KEY), None)
            .verify(&headers, body, FEISHU_TOKEN)
            .is_ok()
    );
    assert!(matches!(
        FeishuAdaptor::configured(None, None).verify(&headers, body, FEISHU_TOKEN),
        Err(WebhookError::Unauthorized(_))
    ));
}

#[test]
fn feishu_ignores_event() {
    let body = feishu_payload("user", "ou_alice", "im.chat.updated_v1");
    let event = expect_event(
        FeishuAdaptor::configured(None, None)
            .normalize(&HeaderMap::new(), body.as_bytes(), FEISHU_TOKEN, "chat")
            .expect("normalize"),
    );
    assert!(!event.in_scope);
}

fn expect_event(outcome: WebhookOutcome) -> NormalizedEvent {
    match outcome {
        WebhookOutcome::Event(event) => event,
        WebhookOutcome::Reply(value) => panic!("expected event, got {value}"),
    }
}

fn sign(secret: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

fn github_headers(event: &str, signature: Option<&str>) -> HeaderMap {
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

fn github_body() -> &'static [u8] {
    br#"{
        "action":"created",
        "repository":{"full_name":"PerishCode/santi"},
        "issue":{"number":42,"title":"title"},
        "comment":{"body":"top secret","html_url":"https://github.com/PerishCode/santi/issues/42#issuecomment-1"},
        "sender":{"login":"somehuman"}
    }"#
}

fn feishu_payload(sender_type: &str, open_id: &str, event_type: &str) -> String {
    format!(
        r#"{{
            "schema":"2.0",
            "header":{{"event_id":"ev_123","event_type":"{event_type}","token":"{FEISHU_TOKEN}"}},
            "event":{{
                "sender":{{"sender_type":"{sender_type}","sender_id":{{"open_id":"{open_id}","user_id":"u_1"}}}},
                "message":{{"chat_id":"oc_chat1","chat_type":"p2p","message_id":"om_msg1","content":"top secret"}}
            }}
        }}"#
    )
}

fn encrypt(plaintext: &str) -> String {
    let key = Sha256::digest(FEISHU_KEY.as_bytes());
    let iv = [7u8; 16];
    let ciphertext = cbc::Encryptor::<aes::Aes256>::new_from_slices(&key, &iv)
        .unwrap()
        .encrypt_padded_vec_mut::<Pkcs7>(plaintext.as_bytes());
    let mut data = iv.to_vec();
    data.extend_from_slice(&ciphertext);
    base64::engine::general_purpose::STANDARD.encode(data)
}
