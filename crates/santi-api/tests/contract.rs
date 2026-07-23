use sha2::{Digest, Sha256};

#[test]
fn api() {
    let text = santi_api::export_openapi_json().expect("export openapi");
    let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    assert_eq!(
        hash,
        "fe40c1fba71e67d4f9da60a5e157e7b8b35b1a14805eb740d81c8a5ceb6b8076"
    );
}
