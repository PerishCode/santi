use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use std::{env, fs, thread};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::artifacts::{Artifact, Dpkg, Store, write_json_atomic};
use super::finalize::{FINALIZE_PROTOCOL_VERSION, finalize_at, persistence_error};
use super::{
    RecoveryStatus, UpgradeFailure, UpgradeFinalizeReport, UpgradeFinalizeRequest, UpgradeHost,
    UpgradeReadiness, UpgradeReport, UpgradeStage, UpgradeStarted, UpgradeTerminal,
    run_upgrade_attempt, upgrade_timeout,
};
use crate::config::RuntimePaths;

mod probe;

use probe::{final_version_binary, probe_final_version_storage, probe_runtime_readiness};

const SANTI_SERVICE: &str = "santi.service";
const UPGRADE_SERVICE: &str = "santi-upgrade.service";
const UPGRADE_REQUEST_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Launch {
    protocol_version: u32,
    attempt_id: String,
    candidate: Artifact,
    previous: Artifact,
}

fn request_path(paths: &RuntimePaths) -> PathBuf {
    paths.runtime_root.join("upgrade.request")
}

pub fn launch(
    deb: &str,
    previous_deb: Option<&str>,
) -> Result<UpgradeStarted, Box<santi_core::Fault>> {
    let paths = crate::runtime::held().paths.clone();
    let attempt_id = format!("upgrade_{}", Uuid::new_v4().simple());
    ensure_upgrade_idle()
        .map_err(|detail| paths.record_failure(&attempt_id, deb, UpgradeStage::Launch, detail))?;
    let previous_deb = supplied_previous(previous_deb);
    let request_body = prepare_request(&paths, &attempt_id, deb, previous_deb.as_deref())
        .map_err(|detail| paths.record_failure(&attempt_id, deb, UpgradeStage::Launch, detail))?;
    let status = Command::new("sudo")
        .args(["-n", "systemctl", "start", "--no-block", UPGRADE_SERVICE])
        .status()
        .map_err(|error| {
            paths.record_failure(
                &attempt_id,
                deb,
                UpgradeStage::Launch,
                format!("sudo -n systemctl start {UPGRADE_SERVICE}: {error}"),
            )
        })?;
    if !status.success() {
        return Err(paths.record_failure(
            &attempt_id,
            deb,
            UpgradeStage::Launch,
            format!("sudo -n systemctl start {UPGRADE_SERVICE} failed"),
        ));
    }
    Ok(UpgradeStarted {
        attempt_id: request_body.attempt_id,
        status: "started",
        timeout_secs: upgrade_timeout().as_secs(),
        log_hint: format!("journalctl -u {UPGRADE_SERVICE} -f"),
        candidate_version: request_body.candidate.version,
        candidate_sha256: request_body.candidate.sha256,
        previous_version: request_body.previous.version,
        previous_sha256: request_body.previous.sha256,
    })
}

pub fn run(
    deb: Option<String>,
    previous_deb: Option<String>,
) -> Result<UpgradeReport, Box<santi_core::Fault>> {
    let paths = crate::runtime::held().paths.clone();
    let request = match deb {
        Some(deb) => {
            let attempt_id = format!("upgrade_{}", Uuid::new_v4().simple());
            let previous_deb = supplied_previous(previous_deb.as_deref());
            prepare_request(&paths, &attempt_id, &deb, previous_deb.as_deref()).map_err(
                |detail| {
                    paths.record_failure(&attempt_id, &deb, UpgradeStage::ResolveRequest, detail)
                },
            )?
        }
        None => read_request(&paths)?,
    };
    if request.protocol_version != UPGRADE_REQUEST_VERSION {
        return Err(paths.record_failure(
            &request.attempt_id,
            "<retained-candidate>",
            UpgradeStage::ResolveRequest,
            format!(
                "invalid upgrade request protocol={}",
                request.protocol_version
            ),
        ));
    }
    let store = Store::new(&paths.runtime_root);
    let probe = Dpkg;
    let durable_previous = store.resolve_previous(None, &probe).map_err(|detail| {
        paths.record_failure(
            &request.attempt_id,
            "<retained-candidate>",
            UpgradeStage::ResolveRequest,
            detail,
        )
    })?;
    if durable_previous != request.previous {
        return Err(paths.record_failure(
            &request.attempt_id,
            "<retained-candidate>",
            UpgradeStage::ResolveRequest,
            "upgrade request previous package does not match durable installed manifest"
                .to_string(),
        ));
    }
    let candidate_path = store.verify(&request.candidate, &probe).map_err(|detail| {
        paths.record_failure(
            &request.attempt_id,
            "<retained-candidate>",
            UpgradeStage::ResolveRequest,
            detail,
        )
    })?;
    store.verify(&request.previous, &probe).map_err(|detail| {
        paths.record_failure(
            &request.attempt_id,
            "<retained-candidate>",
            UpgradeStage::ResolveRequest,
            detail,
        )
    })?;
    let candidate_path = candidate_path.to_str().ok_or_else(|| {
        paths.record_failure(
            &request.attempt_id,
            "<retained-candidate>",
            UpgradeStage::ResolveRequest,
            "retained candidate package path is not valid UTF-8".to_string(),
        )
    })?;
    let mut host = Host::new(
        paths,
        store,
        request.candidate.clone(),
        request.previous.clone(),
    );
    run_upgrade_attempt(
        &mut host,
        candidate_path,
        upgrade_timeout(),
        request.attempt_id,
    )
}

fn prepare_request(
    paths: &RuntimePaths,
    attempt_id: &str,
    deb: &str,
    previous_deb: Option<&str>,
) -> Result<Launch, String> {
    let store = Store::new(&paths.runtime_root);
    let probe = Dpkg;
    let previous = store.resolve_previous(previous_deb.map(Path::new), &probe)?;
    let candidate = store.stage(Path::new(deb), &probe)?;
    let request = Launch {
        protocol_version: UPGRADE_REQUEST_VERSION,
        attempt_id: attempt_id.to_string(),
        candidate,
        previous,
    };
    write_json_atomic(&request_path(paths), &request)?;
    Ok(request)
}

fn ensure_upgrade_idle() -> Result<(), String> {
    let output = Command::new("systemctl")
        .args(["show", UPGRADE_SERVICE, "--property=ActiveState", "--value"])
        .output()
        .map_err(|error| format!("query {UPGRADE_SERVICE} state: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "query {UPGRADE_SERVICE} state failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let state = String::from_utf8_lossy(&output.stdout).trim().to_string();
    match state.as_str() {
        "inactive" | "failed" => Ok(()),
        _ => Err(format!(
            "another upgrade is not terminal: {UPGRADE_SERVICE} active_state={state}"
        )),
    }
}

fn read_request(paths: &RuntimePaths) -> Result<Launch, Box<santi_core::Fault>> {
    let attempt_id = format!("upgrade_{}", Uuid::new_v4().simple());
    let raw = fs::read(request_path(paths)).map_err(|error| {
        paths.record_failure(
            &attempt_id,
            "<unresolved>",
            UpgradeStage::ResolveRequest,
            format!("read upgrade request: {error}"),
        )
    })?;
    serde_json::from_slice(&raw).map_err(|error| {
        paths.record_failure(
            &attempt_id,
            "<unresolved>",
            UpgradeStage::ResolveRequest,
            format!("decode upgrade request: {error}"),
        )
    })
}

fn normalize_path(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn supplied_previous(value: Option<&str>) -> Option<String> {
    normalize_path(value).or_else(|| {
        env::var("SANTI_PREVIOUS_DEB")
            .ok()
            .and_then(|value| normalize_path(Some(&value)))
    })
}

impl RuntimePaths {
    fn record_failure(
        &self,
        attempt_id: &str,
        deb: &str,
        stage: UpgradeStage,
        detail: String,
    ) -> Box<santi_core::Fault> {
        let finalize_request = UpgradeFinalizeRequest {
            protocol_version: FINALIZE_PROTOCOL_VERSION,
            attempt_id: attempt_id.to_string(),
            deb: deb.to_string(),
            terminal: UpgradeTerminal::Failed {
                failure: UpgradeFailure {
                    stage,
                    detail,
                    recovery: RecoveryStatus::Unknown,
                },
            },
            readiness: UpgradeReadiness::Ready,
            wake: false,
            soul_id: santi_core::DEFAULT_SOUL_ID.to_string(),
            configured_strand_id: None,
        };
        match finalize_at(self, finalize_request) {
            Ok(report) => Box::new(report.errors.into_iter().next().unwrap_or_else(|| {
                persistence_error(
                    attempt_id,
                    deb,
                    stage.operation(),
                    "finalizer returned no launch failure incident",
                )
            })),
            Err(error) => error,
        }
    }
}

mod host;
use host::*;
