use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use std::{env, fs, thread};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::finalize::{FINALIZE_PROTOCOL_VERSION, finalize_at, persistence_error};
use super::{
    RecoveryStatus, UpgradeFailure, UpgradeFinalizeReport, UpgradeFinalizeRequest, UpgradeHost,
    UpgradeReport, UpgradeStage, UpgradeStarted, UpgradeTerminal, run_upgrade_attempt,
    upgrade_timeout,
};
use crate::config::{self, RuntimePaths};

const SANTI_SERVICE: &str = "santi.service";
const UPGRADE_SERVICE: &str = "santi-upgrade.service";
const UPGRADE_REQUEST_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UpgradeLaunchRequest {
    protocol_version: u32,
    attempt_id: String,
    deb: String,
}

fn request_path(paths: &RuntimePaths) -> PathBuf {
    paths.runtime_root.join("upgrade.request")
}

/// Record the request and kick the detached oneshot unit, then return fast.
pub fn launch(deb: &str) -> Result<UpgradeStarted, Box<santi_core::SantiError>> {
    let paths = config::resolve_runtime_paths();
    let request_body = UpgradeLaunchRequest {
        protocol_version: UPGRADE_REQUEST_VERSION,
        attempt_id: format!("upgrade_{}", Uuid::new_v4().simple()),
        deb: deb.to_string(),
    };
    let request = request_path(&paths);
    if let Some(parent) = request.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            record_failure(
                &paths,
                &request_body,
                UpgradeStage::Launch,
                format!("create upgrade request directory: {error}"),
            )
        })?;
    }
    let payload = serde_json::to_vec(&request_body).map_err(|error| {
        record_failure(
            &paths,
            &request_body,
            UpgradeStage::Launch,
            format!("encode upgrade request: {error}"),
        )
    })?;
    fs::write(&request, payload).map_err(|error| {
        record_failure(
            &paths,
            &request_body,
            UpgradeStage::Launch,
            format!("write upgrade request: {error}"),
        )
    })?;
    let status = Command::new("sudo")
        .args(["-n", "systemctl", "start", "--no-block", UPGRADE_SERVICE])
        .status()
        .map_err(|error| {
            record_failure(
                &paths,
                &request_body,
                UpgradeStage::Launch,
                format!("sudo -n systemctl start {UPGRADE_SERVICE}: {error}"),
            )
        })?;
    if !status.success() {
        return Err(record_failure(
            &paths,
            &request_body,
            UpgradeStage::Launch,
            format!("sudo -n systemctl start {UPGRADE_SERVICE} failed"),
        ));
    }
    Ok(UpgradeStarted {
        attempt_id: request_body.attempt_id,
        status: "started",
        timeout_secs: upgrade_timeout().as_secs(),
        log_hint: format!("journalctl -u {UPGRADE_SERVICE} -f"),
    })
}

/// Resolve a direct artifact or detached request, then run the orchestration.
pub fn run(deb: Option<String>) -> Result<UpgradeReport, Box<santi_core::SantiError>> {
    let paths = config::resolve_runtime_paths();
    let request = match deb {
        Some(deb) => UpgradeLaunchRequest {
            protocol_version: UPGRADE_REQUEST_VERSION,
            attempt_id: format!("upgrade_{}", Uuid::new_v4().simple()),
            deb,
        },
        None => read_request(&paths)?,
    };
    if request.protocol_version != UPGRADE_REQUEST_VERSION || request.deb.trim().is_empty() {
        return Err(record_failure(
            &paths,
            &request,
            UpgradeStage::ResolveRequest,
            format!(
                "invalid upgrade request: protocol={}, artifact_empty={}",
                request.protocol_version,
                request.deb.trim().is_empty()
            ),
        ));
    }
    let mut host = SystemHost::new(paths);
    run_upgrade_attempt(
        &mut host,
        &request.deb,
        upgrade_timeout(),
        request.attempt_id,
    )
}

fn read_request(paths: &RuntimePaths) -> Result<UpgradeLaunchRequest, Box<santi_core::SantiError>> {
    let unresolved = || UpgradeLaunchRequest {
        protocol_version: UPGRADE_REQUEST_VERSION,
        attempt_id: format!("upgrade_{}", Uuid::new_v4().simple()),
        deb: "<unresolved>".to_string(),
    };
    let raw = fs::read(request_path(paths)).map_err(|error| {
        let request = unresolved();
        record_failure(
            paths,
            &request,
            UpgradeStage::ResolveRequest,
            format!("read upgrade request: {error}"),
        )
    })?;
    serde_json::from_slice(&raw).map_err(|error| {
        let request = unresolved();
        record_failure(
            paths,
            &request,
            UpgradeStage::ResolveRequest,
            format!("decode upgrade request: {error}"),
        )
    })
}

fn record_failure(
    paths: &RuntimePaths,
    request: &UpgradeLaunchRequest,
    stage: UpgradeStage,
    detail: String,
) -> Box<santi_core::SantiError> {
    let finalize_request = UpgradeFinalizeRequest {
        protocol_version: FINALIZE_PROTOCOL_VERSION,
        attempt_id: request.attempt_id.clone(),
        deb: request.deb.clone(),
        terminal: UpgradeTerminal::Failed {
            failure: UpgradeFailure {
                stage,
                detail,
                recovery: RecoveryStatus::Unknown,
            },
        },
        wake: false,
        soul_id: santi_core::DEFAULT_SOUL_ID.to_string(),
        configured_strand_id: None,
    };
    match finalize_at(paths, finalize_request) {
        Ok(report) => Box::new(report.errors.into_iter().next().unwrap_or_else(|| {
            persistence_error(
                &request.attempt_id,
                &request.deb,
                stage.operation(),
                "finalizer returned no launch failure incident",
            )
        })),
        Err(error) => error,
    }
}

struct SystemHost {
    paths: RuntimePaths,
    backup: PathBuf,
}

impl SystemHost {
    fn new(paths: RuntimePaths) -> Self {
        let backup = paths
            .runtime_root
            .with_file_name("santi-runtime-backup.tar.gz");
        Self { paths, backup }
    }

    fn privileged(&self, args: &[&str]) -> Result<(), String> {
        let status = Command::new("sudo")
            .arg("-n")
            .args(args)
            .status()
            .map_err(|error| format!("sudo -n {}: {error}", args.join(" ")))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("sudo -n {} failed", args.join(" ")))
        }
    }

    fn systemctl(&self, action: &str) -> Result<(), String> {
        self.privileged(&["systemctl", action, SANTI_SERVICE])
    }
}

impl UpgradeHost for SystemHost {
    fn graceful_stop(&mut self, _grace: Duration) -> Result<(), String> {
        self.systemctl("stop")
    }

    fn snapshot(&mut self) -> Result<(), String> {
        let root = &self.paths.runtime_root;
        let parent = root.parent().ok_or("runtime_root has no parent")?;
        let name = root.file_name().ok_or("runtime_root has no name")?;
        let status = Command::new("tar")
            .arg("czf")
            .arg(&self.backup)
            .arg("-C")
            .arg(parent)
            .arg(name)
            .status()
            .map_err(|error| format!("tar snapshot: {error}"))?;
        if status.success() {
            Ok(())
        } else {
            Err("runtime snapshot (tar) failed".to_string())
        }
    }

    fn install(&mut self, deb: &str) -> Result<(), String> {
        self.privileged(&["dpkg", "-i", deb])
    }

    fn trial_probe(&mut self) -> Result<bool, String> {
        self.systemctl("start")?;
        let deadline = Instant::now() + upgrade_timeout();
        let healthy = loop {
            match crate::ops::doctor_at(&self.paths) {
                Ok(report) if report.ok => break true,
                _ if Instant::now() >= deadline => break false,
                _ => thread::sleep(Duration::from_millis(500)),
            }
        };
        self.systemctl("stop")?;
        Ok(healthy)
    }

    fn rollback(&mut self) -> Result<(), String> {
        let parent = self
            .paths
            .runtime_root
            .parent()
            .ok_or("runtime_root has no parent")?;
        let status = Command::new("tar")
            .arg("xzf")
            .arg(&self.backup)
            .arg("-C")
            .arg(parent)
            .status()
            .map_err(|error| format!("tar restore: {error}"))?;
        if !status.success() {
            return Err("runtime restore (tar) failed".to_string());
        }
        if let Ok(prev) = env::var("SANTI_PREVIOUS_DEB").map(|value| value.trim().to_string())
            && !prev.is_empty()
        {
            self.install(&prev)?;
        }
        Ok(())
    }

    fn finalize(
        &mut self,
        request: &UpgradeFinalizeRequest,
    ) -> Result<UpgradeFinalizeReport, String> {
        let binary = env::var("SANTI_UPGRADE_FINALIZER_BIN")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "/usr/bin/santi".to_string());
        let mut child = Command::new(&binary)
            .args(["upgrade", "--finalize"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("spawn final-version binary {binary}: {error}"))?;
        let mut request = request.clone();
        request.soul_id = env::var("SANTI_SOUL_ID")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| santi_core::DEFAULT_SOUL_ID.to_string());
        request.configured_strand_id = env::var("SANTI_STRAND_ID")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let payload = serde_json::to_vec(&request).map_err(|error| error.to_string())?;
        child
            .stdin
            .take()
            .ok_or("final-version binary stdin unavailable")?
            .write_all(&payload)
            .map_err(|error| format!("write finalization request: {error}"))?;
        let output = child
            .wait_with_output()
            .map_err(|error| format!("wait for final-version binary: {error}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "final-version binary exited with {}: {}",
                output.status,
                stderr.trim()
            ));
        }
        serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("decode final-version report: {error}"))
    }

    fn start(&mut self) -> Result<(), String> {
        self.systemctl("start")
    }
}
