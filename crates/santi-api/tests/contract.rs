use sha2::{Digest, Sha256};

#[test]
fn api() {
    let text = santi_api::export_openapi_json().expect("export openapi");
    let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    assert_eq!(
        hash,
        "ce9829d39dd8cd072c24fb7ef5be91d6b1eb56e5f6be21a7f442b153c499c9f6"
    );
}
