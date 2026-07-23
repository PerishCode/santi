use sha2::{Digest, Sha256};

#[test]
fn api() {
    let text = santi_api::export_openapi_json().expect("export openapi");
    let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    assert_eq!(
        hash,
        "c69d69855a3353e52b163cdf68d79534b5a47c163ed66ca1d9b8d3b1147d37a7"
    );
}
