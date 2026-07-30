use std::{
    fmt,
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer as _, SigningKey};
use serde::Serialize;

const PREFIX: &str = "santi1";
const SCHEMA: &str = "santi.runtime-capability.v1";
const LIMIT: u64 = 300;

pub struct Key<'a> {
    pub id: &'a str,
    pub private: &'a str,
}

#[derive(Clone)]
pub struct Issuer {
    issuer: String,
    audience: String,
    key_id: String,
    ttl: u64,
    key: SigningKey,
}

#[derive(Clone, Copy)]
pub struct Origin<'a> {
    pub soul: &'a str,
    pub strand: &'a str,
    pub turn: &'a str,
    pub call: &'a str,
    pub effect: &'a str,
}

#[derive(Serialize)]
struct Claims<'a> {
    schema: &'static str,
    iss: &'a str,
    aud: &'a str,
    kid: &'a str,
    soul: &'a str,
    strand: &'a str,
    turn: &'a str,
    call: &'a str,
    effect: &'a str,
    iat: u64,
    exp: u64,
}

impl Issuer {
    pub fn new(
        issuer: &str,
        audience: &str,
        key: Key<'_>,
        ttl_seconds: u64,
    ) -> Result<Self, String> {
        let issuer = required("capability.issuer", issuer, 256)?;
        let audience = required("capability.audience", audience, 256)?;
        let key_id = required("capability.key_id", key.id, 128)?;
        if ttl_seconds == 0 || ttl_seconds > LIMIT {
            return Err(format!(
                "capability.ttl_seconds must be between 1 and {LIMIT}"
            ));
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(key.private.trim())
            .map_err(|_| "capability.private_key is not unpadded base64url".to_string())?;
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| "capability.private_key must decode to 32 bytes".to_string())?;
        Ok(Self {
            issuer,
            audience,
            key_id,
            ttl: ttl_seconds,
            key: SigningKey::from_bytes(&bytes),
        })
    }

    pub fn issue(&self, origin: Origin<'_>) -> Result<String, String> {
        self.mint(origin, epoch()?)
    }

    pub fn public(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.key.verifying_key().as_bytes())
    }

    pub fn id(&self) -> &str {
        &self.key_id
    }

    fn mint(&self, origin: Origin<'_>, now: u64) -> Result<String, String> {
        bounded("capability soul", origin.soul, 256)?;
        bounded("capability strand", origin.strand, 256)?;
        bounded("capability turn", origin.turn, 256)?;
        bounded("capability tool call", origin.call, 256)?;
        bounded("capability effect", origin.effect, 256)?;
        let claims = Claims {
            schema: SCHEMA,
            iss: &self.issuer,
            aud: &self.audience,
            kid: &self.key_id,
            soul: origin.soul,
            strand: origin.strand,
            turn: origin.turn,
            call: origin.call,
            effect: origin.effect,
            iat: now,
            exp: now
                .checked_add(self.ttl)
                .ok_or_else(|| "capability expiry is out of range".to_string())?,
        };
        let payload = serde_json::to_vec(&claims).map_err(|error| error.to_string())?;
        let payload = URL_SAFE_NO_PAD.encode(payload);
        let signed = format!("{PREFIX}.{payload}");
        let signature = self.key.sign(signed.as_bytes());
        Ok(format!(
            "{signed}.{}",
            URL_SAFE_NO_PAD.encode(signature.to_bytes())
        ))
    }
}

impl fmt::Debug for Issuer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Issuer")
            .field("issuer", &self.issuer)
            .field("audience", &self.audience)
            .field("key_id", &self.key_id)
            .field("ttl", &self.ttl)
            .field("private_key", &"[redacted]")
            .finish()
    }
}

fn required(name: &str, value: &str, limit: usize) -> Result<String, String> {
    let value = value.trim();
    bounded(name, value, limit)?;
    Ok(value.to_string())
}

fn bounded(name: &str, value: &str, limit: usize) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{name} must not be empty"));
    }
    if value.len() > limit {
        return Err(format!("{name} must not exceed {limit} bytes"));
    }
    Ok(())
}

fn epoch() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| error.to_string())
}
