use std::time::{SystemTime, UNIX_EPOCH};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signature, Verifier as _, VerifyingKey};
use santi_core::capability::{Issuer, Key, Origin};
use serde_json::Value;

#[test]
fn signs() {
    let private = URL_SAFE_NO_PAD.encode([7; 32]);
    let issuer = issuer(&private, 120).unwrap();
    let token = issuer.issue(origin()).unwrap();
    let mut parts = token.split('.');
    assert_eq!(parts.next(), Some("santi1"));
    let payload = parts.next().unwrap();
    let signature = parts.next().unwrap();
    assert!(parts.next().is_none());
    let claims: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap();
    assert_eq!(claims["schema"], "santi.runtime-capability.v1");
    assert_eq!(claims["iss"], "santi.example");
    assert_eq!(claims["aud"], "stim.reply");
    assert_eq!(claims["kid"], "test-2026");
    assert_eq!(claims["soul"], "soul_1");
    assert_eq!(claims["strand"], "strand_1");
    assert_eq!(claims["turn"], "turn_1");
    assert_eq!(claims["call"], "call_1");
    assert_eq!(claims["effect"], "effect_1");
    let issued = claims["iat"].as_u64().unwrap();
    assert_eq!(claims["exp"].as_u64().unwrap(), issued + 120);
    assert!(issued <= epoch());
    let public: [u8; 32] = URL_SAFE_NO_PAD
        .decode(issuer.public())
        .unwrap()
        .try_into()
        .unwrap();
    let signature = Signature::from_slice(&URL_SAFE_NO_PAD.decode(signature).unwrap()).unwrap();
    VerifyingKey::from_bytes(&public)
        .unwrap()
        .verify(format!("santi1.{payload}").as_bytes(), &signature)
        .unwrap();
    assert_eq!(issuer.id(), "test-2026");
    assert!(!format!("{issuer:?}").contains(&private));
}

#[test]
fn refuses() {
    let private = URL_SAFE_NO_PAD.encode([7; 32]);
    assert!(Issuer::new("", "stim.reply", key(&private), 120).is_err());
    assert!(issuer(&private, 0).is_err());
    assert!(issuer(&private, 301).is_err());
    assert!(issuer("not-a-key", 120).is_err());
}

fn issuer(private: &str, ttl: u64) -> Result<Issuer, String> {
    Issuer::new("santi.example", "stim.reply", key(private), ttl)
}

fn key(private: &str) -> Key<'_> {
    Key {
        id: "test-2026",
        private,
    }
}

fn origin() -> Origin<'static> {
    Origin {
        soul: "soul_1",
        strand: "strand_1",
        turn: "turn_1",
        call: "call_1",
        effect: "effect_1",
    }
}

fn epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
