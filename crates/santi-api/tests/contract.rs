use sha2::{Digest, Sha256};

#[test]
fn api() {
    let text = santi_api::export_openapi_json().expect("export openapi");
    let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    assert_eq!(
        hash,
        "142292d57773e6970be50efb05eb7ac56a848da7000c6a8146279a5e4f23a23a"
    );
}
