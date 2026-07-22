use sha2::{Digest, Sha256};

#[test]
fn api() {
    let text = santi_api::export_openapi_json().expect("export openapi");
    let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    assert_eq!(
        hash,
        "9d762a0ac277beca61a80588a5181c618f5c4cc89e71cafcf165911dd892d25e"
    );
}
