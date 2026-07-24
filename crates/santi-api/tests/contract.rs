use sha2::{Digest, Sha256};

#[test]
fn api() {
    let text = santi_api::export_openapi_json().expect("export openapi");
    let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    assert_eq!(
        hash,
        "7cc5c751212b72601eb56992ea0997f499e4b3e7025505935daa4a62f4c781b0"
    );
}
