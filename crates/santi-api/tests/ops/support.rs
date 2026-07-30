pub(super) async fn bootstrap(path: &std::path::Path) -> santi_core::Store {
    santi_core::Store::bootstrap(
        path,
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    )
    .await
    .expect("bootstrap")
}
