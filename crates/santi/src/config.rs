use std::path::PathBuf;

pub fn load() {
    dotenvy::dotenv().ok();
}

pub fn env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub fn shelter() -> PathBuf {
    env("HOME")
        .map(|home| PathBuf::from(home).join(".cache/santi"))
        .unwrap_or_else(std::env::temp_dir)
}
