use sha2::{Digest, Sha256};

#[test]
fn api() {
    let text = santi_api::export_openapi_json().expect("export openapi");
    let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    assert_eq!(
        hash,
        "4a20f232b869323022a254b1cebdce2d01cb33eb3685f3e94612a40ca87dcda6"
    );
}
