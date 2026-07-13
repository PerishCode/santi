use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::super::UpgradeReadiness;

const DEFAULT_FINAL_VERSION_BINARY: &str = "/usr/bin/santi";

// dpkg replaces the runner's inode, so current_exe() may resolve to a deleted
// path after install. Probes and finalization must execute the installed binary.
pub(super) fn final_version_binary() -> PathBuf {
    let configured = env::var("SANTI_UPGRADE_FINALIZER_BIN").ok();
    resolve_final_version_binary(configured.as_deref())
}

fn resolve_final_version_binary(configured: Option<&str>) -> PathBuf {
    configured
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_FINAL_VERSION_BINARY))
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
    let port = env::var("SANTI_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(43307);
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
