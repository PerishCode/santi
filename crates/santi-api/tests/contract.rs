use sha2::{Digest, Sha256};

#[test]
fn api() {
    let text = santi_api::export_openapi_json().expect("export openapi");
    let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    assert_eq!(
        hash,
        "1eddef8da1e8109e1017d8cff356c50876384384f3a24fd462806ff90e5691ed"
    );
}
