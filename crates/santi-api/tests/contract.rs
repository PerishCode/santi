use sha2::{Digest, Sha256};

#[test]
fn api() {
    let text = santi_api::export_openapi_json().expect("export openapi");
    let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    assert_eq!(
        hash,
        "3173924caad56b6ed5e455786d69ca1c49bb526191324cce0b808f62c396494f"
    );
}
