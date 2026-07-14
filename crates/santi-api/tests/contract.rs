use sha2::{Digest, Sha256};

#[test]
fn api() {
    let text = santi_api::export_openapi_json().expect("export openapi");
    let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    assert_eq!(
        hash,
        "3218ee3f9c5f77f8a0c520821b43f83938ca22b8da6b67df263afb532b9514fd"
    );
}
