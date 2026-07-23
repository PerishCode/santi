use sha2::{Digest, Sha256};

#[test]
fn api() {
    let text = santi_api::export_openapi_json().expect("export openapi");
    let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    assert_eq!(
        hash,
        "30d19b071fae3e67a6dea77e391ec46172dbecb85fc22fb21fd49084e72e249c"
    );
}
