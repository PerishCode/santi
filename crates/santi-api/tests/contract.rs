use sha2::{Digest, Sha256};

#[test]
fn api() {
    let text = santi_api::export_openapi_json().expect("export openapi");
    let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    assert_eq!(
        hash,
        "44d847caa42798844f33ff6f482544b0e2c5f904252f924c497f1ae77c5cb9cc"
    );
}
