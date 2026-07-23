use sha2::{Digest, Sha256};

#[test]
fn api() {
    let text = santi_api::export_openapi_json().expect("export openapi");
    let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    assert_eq!(
        hash,
        "86bfc842d5ca973fa5357ef8a9f9c17b05ce936c2d424fbc67dc6f117c17c462"
    );
}
