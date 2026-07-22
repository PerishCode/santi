use sha2::{Digest, Sha256};

#[test]
fn api() {
    let text = santi_api::export_openapi_json().expect("export openapi");
    let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    assert_eq!(
        hash,
        "68ec0801b4de6e2a6298b2a2f06df7c95e9476907faf6e98ef3454bbc28e132c"
    );
}
