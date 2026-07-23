use sha2::{Digest, Sha256};

#[test]
fn api() {
    let text = santi_api::export_openapi_json().expect("export openapi");
    let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    assert_eq!(
        hash,
        "6ff5be846003bdf3be5021284f5b1946a8481cb58de7f4af8862f1db0ad13ad6"
    );
}
