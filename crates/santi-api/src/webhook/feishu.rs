use std::env;

use axum::http::HeaderMap;
use base64::Engine as _;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{
    NormalizedEvent, WebhookAdaptor, WebhookError, WebhookOutcome, sender_allowed, string_at,
};

const ENCRYPT_KEY_ENV: &str = "SANTI_WEBHOOK_FEISHU_ENCRYPT_KEY";
const ALLOW_ENV: &str = "SANTI_WEBHOOK_FEISHU_ALLOW";

pub struct FeishuAdaptor {
    encrypt_key: Option<String>,
    allow: Option<String>,
}

impl FeishuAdaptor {
    pub fn configured(encrypt_key: Option<&str>, allow: Option<&str>) -> Self {
        Self {
            encrypt_key: normalized(encrypt_key),
            allow: normalized(allow),
        }
    }

    pub(crate) fn from_env() -> Self {
        Self::configured(
            env::var(ENCRYPT_KEY_ENV).ok().as_deref(),
            env::var(ALLOW_ENV).ok().as_deref(),
        )
    }
}

impl WebhookAdaptor for FeishuAdaptor {
    fn verify(
        &self,
        headers: &HeaderMap,
        raw_body: &[u8],
        _secret: &str,
    ) -> Result<(), WebhookError> {
        verify_signature(headers, raw_body, self.encrypt_key.as_deref())
    }

    fn normalize(
        &self,
        _headers: &HeaderMap,
        raw_body: &[u8],
        secret: &str,
        webhook_name: &str,
    ) -> Result<WebhookOutcome, WebhookError> {
        let raw: Value = serde_json::from_slice(raw_body)
            .map_err(|error| WebhookError::BadRequest(format!("invalid JSON body: {error}")))?;
        let payload = match raw.get("encrypt").and_then(Value::as_str) {
            Some(ciphertext) => {
                let key = self.encrypt_key.as_deref().ok_or_else(|| {
                    WebhookError::Unauthorized(format!(
                        "encrypted payload but {ENCRYPT_KEY_ENV} is not set"
                    ))
                })?;
                let plaintext = decrypt(key, ciphertext)?;
                serde_json::from_slice::<Value>(&plaintext).map_err(|error| {
                    WebhookError::BadRequest(format!("invalid decrypted JSON: {error}"))
                })?
            }
            None => raw,
        };

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

        if payload.get("type").and_then(Value::as_str) == Some("url_verification") {
            let challenge = payload
                .get("challenge")
                .and_then(Value::as_str)
                .unwrap_or("");
            return Ok(WebhookOutcome::Reply(json!({ "challenge": challenge })));
        }

        let event_type = payload
            .pointer("/header/event_type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if event_type != "im.message.receive_v1" {
            return Ok(WebhookOutcome::Event(NormalizedEvent {
                santi_system_text: String::new(),
                label: format!("feishu:{webhook_name}:{event_type}"),
                metadata: None,
                in_scope: false,
                self_authored: false,
            }));
        }

        let chat_id = string_at(&payload, &["event", "message", "chat_id"]).unwrap_or_default();
        let chat_type = string_at(&payload, &["event", "message", "chat_type"]).unwrap_or_default();
        let message_id =
            string_at(&payload, &["event", "message", "message_id"]).unwrap_or_default();
        let event_id = string_at(&payload, &["header", "event_id"]).unwrap_or_default();
        let sender_type =
            string_at(&payload, &["event", "sender", "sender_type"]).unwrap_or_default();
        let open_id =
            string_at(&payload, &["event", "sender", "sender_id", "open_id"]).unwrap_or_default();
        let user_id =
            string_at(&payload, &["event", "sender", "sender_id", "user_id"]).unwrap_or_default();
        let allowed = sender_allowed(&open_id, self.allow.as_deref())
            || sender_allowed(&user_id, self.allow.as_deref());
        let in_scope = sender_type == "user" && allowed;
        let santi_system_text = format!(
            "[feishu] im.message.receive_v1 in chat {chat_id} ({chat_type})\nmessage_id: {message_id}\nevent_id: {event_id}"
        );
        let label = format!("feishu:{webhook_name}:chat:{chat_id}");
        let metadata = json!({
            "adaptor": "feishu",
            "webhook_name": webhook_name,
            "event_type": event_type,
            "event_id": event_id,
            "chat_id": chat_id,
            "chat_type": chat_type,
            "message_id": message_id,
            "sender_type": sender_type,
            "label": label,
        });

        Ok(WebhookOutcome::Event(NormalizedEvent {
            santi_system_text,
            label,
            metadata: Some(metadata),
            in_scope,
            self_authored: false,
        }))
    }
}

fn verify_signature(
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
        WebhookError::Unauthorized(format!("signed payload but {ENCRYPT_KEY_ENV} is not set"))
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

fn decrypt(encrypt_key: &str, ciphertext_b64: &str) -> Result<Vec<u8>, WebhookError> {
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

fn normalized(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}
