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
use crate::config::{self, RuntimePaths};

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
) -> Result<UpgradeStarted, Box<santi_core::SantiError>> {
    let paths = config::resolve_runtime_paths();
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
) -> Result<UpgradeReport, Box<santi_core::SantiError>> {
    let paths = config::resolve_runtime_paths();
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

fn read_request(paths: &RuntimePaths) -> Result<Launch, Box<santi_core::SantiError>> {
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
    ) -> Box<santi_core::SantiError> {
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

struct Host {
    paths: RuntimePaths,
    backup: PathBuf,
    store: Store,
    candidate: Artifact,
    previous: Artifact,
}

impl Host {
    fn new(paths: RuntimePaths, store: Store, candidate: Artifact, previous: Artifact) -> Self {
        let backup = paths
            .runtime_root
            .with_file_name("santi-runtime-backup.tar.gz");
        Self {
            paths,
            backup,
            store,
            candidate,
            previous,
        }
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

fn probe_readiness(binary: &Path) -> Result<Option<UpgradeReadiness>, String> {
    probe_final_version_storage(binary)?;
    probe_runtime_readiness(binary)
}

impl UpgradeHost for Host {
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

    fn trial_probe(&mut self) -> Result<UpgradeReadiness, String> {
        self.systemctl("start")?;
        let deadline = Instant::now() + upgrade_timeout();
        let mut last_detail = "service health was not reachable".to_string();
        let binary = final_version_binary();
        let readiness = loop {
            match probe_readiness(&binary) {
                Ok(Some(readiness)) => break Ok(readiness),
                Ok(None) => {}
                Err(error) => last_detail = error,
            }
            if Instant::now() >= deadline {
                break Err(format!("trial probe timed out: {last_detail}"));
            }
            thread::sleep(Duration::from_millis(500));
        };
        let stop = self.systemctl("stop");
        match (readiness, stop) {
            (Ok(readiness), Ok(())) => Ok(readiness),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(error)) => Err(format!("trial service stop failed: {error}")),
            (Err(probe_error), Err(stop_error)) => Err(format!(
                "{probe_error}; trial service stop also failed: {stop_error}"
            )),
        }
    }

    fn retain_candidate(&mut self) -> Result<(), String> {
        let probe = Dpkg;
        self.store.commit_installed(&self.candidate, &probe)?;
        self.store.prune(&[&self.candidate, &self.previous])
    }

    fn rollback(&mut self) -> Result<(), String> {
        let probe = Dpkg;
        let previous_deb = self.store.verify(&self.previous, &probe)?;
        let previous_deb = previous_deb
            .to_str()
            .ok_or("retained previous package path is not valid UTF-8")?
            .to_string();
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
        self.install(&previous_deb)?;
        self.store.commit_installed(&self.previous, &probe)?;
        self.store.prune(&[&self.previous, &self.candidate])?;
        Ok(())
    }

    fn finalize(
        &mut self,
        request: &UpgradeFinalizeRequest,
    ) -> Result<UpgradeFinalizeReport, String> {
        let binary = final_version_binary();
        let mut child = Command::new(&binary)
            .args(["upgrade", "--finalize"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("spawn final-version binary {}: {error}", binary.display()))?;
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
