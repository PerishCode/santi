use sha2::{Digest, Sha256};

#[test]
fn api() {
    let text = santi_api::export_openapi_json().expect("export openapi");
    let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    assert_eq!(
        hash,
        "09bd87e34cdb3d76d9cbac2870885e4333259e849c0a3c8599aeb63af7e921fb"
    );
}
