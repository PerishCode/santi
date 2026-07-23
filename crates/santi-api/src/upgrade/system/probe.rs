use std::path::{Path, PathBuf};
use std::process::Command;

use super::super::UpgradeReadiness;

pub(super) fn final_version_binary() -> PathBuf {
    crate::runtime::held().finalizer_bin.clone()
}

pub(super) fn probe_final_version_storage(binary: &Path) -> Result<(), String> {
    let output = Command::new(binary)
        .args(["doctor", "--storage-only"])
        .output()
        .map_err(|error| format!("run final-version storage doctor: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "final-version storage doctor failed with {}: {}{}",
        output.status,
        stdout.trim(),
        stderr.trim()
    ))
}

pub(super) fn probe_runtime_readiness(binary: &Path) -> Result<Option<UpgradeReadiness>, String> {
    let port = crate::runtime::held().listen_port;
    let base_url = format!("http://127.0.0.1:{port}");
    let output = Command::new(binary)
        .args(["--base-url", &base_url, "health"])
        .env_remove("SANTI_AUTH_TOKEN_URL")
        .env_remove("SANTI_AUTH_CLIENT_ID")
        .env_remove("SANTI_AUTH_USERNAME")
        .env_remove("SANTI_AUTH_PASSWORD")
        .env_remove("SANTI_API_KEY")
        .output()
        .map_err(|error| format!("run health probe: {error}"))?;
    if output.stdout.is_empty() {
        return Ok(None);
    }
    let health: santi_core::HealthResponse = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("decode health probe: {error}"))?;
    if health.degraded {
        Ok(Some(UpgradeReadiness::Degraded))
    } else if health.ok {
        Ok(Some(UpgradeReadiness::Ready))
    } else {
        Ok(None)
    }
}
