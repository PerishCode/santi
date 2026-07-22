use sha2::{Digest, Sha256};

#[test]
fn api() {
    let text = santi_api::export_openapi_json().expect("export openapi");
    let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    assert_eq!(
        hash,
        "d56b8cb3149da3f53a963eb60f89e7304f95728d47b4cd16e0dcab3d8e2336f2"
    );
}
