use std::path::Path;

pub(super) async fn bootstrap(path: impl AsRef<Path>) -> santi_estate::Store {
    santi_estate::Store::bootstrap(
        path,
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    )
    .await
    .expect("bootstrap")
}
