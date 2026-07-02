//! `santi upgrade` — self-upgrade orchestration (PHASE-07 STEP 4).
//!
//! Two faces, split by `--run`:
//!
//! - **launcher** (`santi upgrade <deb>`): what the operator — later Liberte —
//!   invokes. It writes the request and kicks the shipped `santi-upgrade.service`
//!   oneshot unit via `systemctl start --no-block`, then returns FAST with a
//!   signal (监听 / 最长超时 Xmin / 日志位置). Because the real work runs as a
//!   systemd unit under PID 1, it is OUTSIDE santi.service's cgroup, so stopping
//!   santi does not kill the upgrader (the self-restart-from-own-cgroup problem).
//! - **runner** (`santi upgrade --run <deb>`): what the oneshot unit executes.
//!   It orchestrates the sequence below, seeding a durable "come look" record
//!   before the final start so boot recovery wakes the soul into the result.
//!
//! Sequence (`run_upgrade`): graceful-stop → snapshot → dpkg → resolve (install +
//! a trial start/health probe) → auto-rollback on a CRISP failure → seed the
//! truthful record (offline, into the FINAL DB) → start the FINAL version.
//!
//! The side effects live behind [`UpgradeHost`] so the orchestration LOGIC here
//! is unit-tested with a fake; the real systemctl/dpkg shell ([`SystemHost`]) is
//! validated on a Debian box (PHASE-07 STEP 6), not in CI.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};
use std::{env, fs, thread};

use serde::Serialize;

use crate::config::{self, RuntimePaths};

/// The managed service unit + the detached oneshot upgrade unit (shipped by the
/// .deb, PHASE-07 STEP 5). The upgrade runs under the oneshot unit — i.e. under
/// PID 1, outside `santi.service`'s cgroup — so stopping santi cannot kill it.
const SANTI_SERVICE: &str = "santi.service";
const UPGRADE_SERVICE: &str = "santi-upgrade.service";

/// Why the runner rolled back to the previous version. Carried into the seeded
/// record because, after a rollback, the failure is NOT observable from the
/// world the soul wakes into (occurrence-not-outcome: preserve what it can't
/// otherwise see).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "cause", content = "detail")]
pub enum RollbackCause {
    InstallFailed(String),
    DidNotComeUp,
}

/// The resolved outcome of an upgrade attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum Outcome {
    /// The new version installed and came up; it is the final version.
    Upgraded,
    /// A crisp pre-commit failure → restored to the previous version.
    RolledBack(RollbackCause),
}

impl Outcome {
    fn is_rollback(&self) -> bool {
        matches!(self, Outcome::RolledBack(_))
    }
}

/// What the runner did, for the log + the launcher's after-the-fact inspection.
#[derive(Debug, Clone, Serialize)]
pub struct UpgradeReport {
    pub outcome: Outcome,
    /// The text seeded into the soul's inbox ("come look" / failure feedback).
    pub record: String,
    /// Whether the record was durably enqueued (false ⟺ no self-strand
    /// configured or the seed was rejected; the upgrade still completes).
    pub seeded: bool,
}

/// The launcher's fast return: "started listening, max timeout, log location".
#[derive(Debug, Clone, Serialize)]
pub struct UpgradeStarted {
    pub status: &'static str,
    pub timeout_secs: u64,
    pub log_hint: String,
}

/// The side effects an upgrade needs, abstracted so the orchestration is
/// testable. Every method is a discrete, ordered step; `run_upgrade` is the only
/// place their order lives.
pub trait UpgradeHost {
    /// Ask the running service to gracefully quiesce + stop within `grace`
    /// (SIGTERM; the service pauses consumption, drains the in-flight turn).
    fn graceful_stop(&mut self, grace: Duration) -> Result<(), String>;
    /// Snapshot the whole runtime (db + souls) so a rollback can restore it.
    fn snapshot(&mut self) -> Result<(), String>;
    /// `dpkg -i` the new package. `Err` ⟺ the install itself failed.
    fn install(&mut self, deb: &str) -> Result<(), String>;
    /// Start the newly-installed version and probe whether it CAME UP (the crisp
    /// soul-deep-adjacent gate: process up + schema migrated + memory readable),
    /// then stop it again so the final start is uniform. `Ok(true)` ⟺ healthy.
    fn trial_probe(&mut self) -> Result<bool, String>;
    /// Restore the snapshot + reinstall the previous version (the final = OLD).
    fn rollback(&mut self) -> Result<(), String>;
    /// Seed one durable "come look" record into the soul's self-strand, offline,
    /// using the FINAL version's binary/schema. Best-effort at the call site.
    fn seed(&mut self, text: &str) -> Result<(), String>;
    /// Start the FINAL version for real (boot recovery then drains the seed).
    fn start(&mut self) -> Result<(), String>;
}

/// Orchestrate one upgrade over `host`. Pure control flow — no I/O of its own —
/// so the branching (success / install-fail / did-not-come-up), the ordering
/// (snapshot before dpkg; seed before the final start), and the record content
/// are all exercised in tests with a fake host.
pub fn run_upgrade<H: UpgradeHost>(
    host: &mut H,
    deb: &str,
    grace: Duration,
) -> Result<UpgradeReport, String> {
    // Quiesce + snapshot BEFORE touching the binary. A failure here is fatal
    // (we must not dpkg over a live/un-snapshotted runtime).
    host.graceful_stop(grace)?;
    host.snapshot()?;

    // Resolve the final version. A crisp failure (install error, or the new
    // version does not come up) routes to rollback; anything else is Upgraded.
    let outcome = match host.install(deb) {
        Err(error) => Outcome::RolledBack(RollbackCause::InstallFailed(error)),
        Ok(()) => match host.trial_probe() {
            Ok(true) => Outcome::Upgraded,
            // A failed probe OR a probe that could not run → conservative
            // rollback (we could not confirm the new version is healthy).
            Ok(false) | Err(_) => Outcome::RolledBack(RollbackCause::DidNotComeUp),
        },
    };

    if outcome.is_rollback() {
        // Rollback failing is break-glass — surface it rather than half-proceed.
        host.rollback()?;
    }

    // Seed the truthful record (offline, into the FINAL DB) BEFORE the final
    // start, so boot recovery is guaranteed to wake the soul into it. Seeding is
    // best-effort: a missing self-strand must not abort an otherwise-done upgrade.
    let record = compose_record(deb, &outcome);
    let seeded = host.seed(&record).is_ok();

    // Start the final version for real.
    host.start()?;

    Ok(UpgradeReport {
        outcome,
        record,
        seeded,
    })
}

/// The occurrence the soul wakes into. Success is minimal ("come look" — she
/// observes the healthy new version herself); failure CARRIES its reason (she
/// wakes on the old version, where the failure is no longer visible).
pub fn compose_record(deb: &str, outcome: &Outcome) -> String {
    match outcome {
        Outcome::Upgraded => format!(
            "You just upgraded santi (from `{deb}`) and it came back up on the new version. \
             Take a look and confirm things are working as you expect."
        ),
        Outcome::RolledBack(RollbackCause::InstallFailed(error)) => format!(
            "An upgrade of santi (from `{deb}`) FAILED while installing and was rolled back — \
             you are on the previous version. The install error was: {error}"
        ),
        Outcome::RolledBack(RollbackCause::DidNotComeUp) => format!(
            "An upgrade of santi (from `{deb}`) was rolled back — the new version did not come \
             up healthy (process/schema/memory check), so you are on the previous version."
        ),
    }
}

/// The max time the whole upgrade may take before the external stop (SIGKILL of
/// the oneshot unit) is the hard bound. `SANTI_UPGRADE_TIMEOUT_SECS`, default 600.
pub fn upgrade_timeout() -> Duration {
    let secs = env::var("SANTI_UPGRADE_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(600);
    Duration::from_secs(secs)
}

fn request_path(paths: &RuntimePaths) -> PathBuf {
    paths.runtime_root.join("upgrade.request")
}

// ─────────────────────────────────────────────────────────────────────────────
// Launcher + runner entrypoints. Below `run_upgrade` (tested above), these are
// the on-box glue: they shell out to systemctl/dpkg and are validated on a
// Debian box in PHASE-07 STEP 6, NOT in CI. Kept thin and explicit so that
// on-box tuning is a matter of adjusting these command lines.
// ─────────────────────────────────────────────────────────────────────────────

/// LAUNCHER (`santi upgrade <deb>`): record the request and kick the detached
/// oneshot unit, then return fast. Does NOT block on the upgrade.
pub fn launch(deb: &str) -> Result<UpgradeStarted, String> {
    let paths = config::resolve_runtime_paths();
    let request = request_path(&paths);
    if let Some(parent) = request.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(&request, deb).map_err(|error| format!("write upgrade request: {error}"))?;
    // `sudo -n`: the launcher is invoked by the santi user (Liberte's shell), and
    // starting a system unit is privileged (santi has passwordless sudo). `--no-block`:
    // return immediately; the oneshot unit runs under PID 1, outside santi's cgroup.
    let status = Command::new("sudo")
        .args(["-n", "systemctl", "start", "--no-block", UPGRADE_SERVICE])
        .status()
        .map_err(|error| format!("sudo -n systemctl start {UPGRADE_SERVICE}: {error}"))?;
    if !status.success() {
        return Err(format!("sudo -n systemctl start {UPGRADE_SERVICE} failed"));
    }
    Ok(UpgradeStarted {
        status: "started",
        timeout_secs: upgrade_timeout().as_secs(),
        log_hint: format!("journalctl -u {UPGRADE_SERVICE} -f"),
    })
}

/// RUNNER (`santi upgrade --run [deb]`): the oneshot unit's body. Resolves the
/// deb (positional, else the request file), then runs the orchestration.
pub fn run(deb: Option<String>) -> Result<UpgradeReport, String> {
    let paths = config::resolve_runtime_paths();
    let deb = match deb {
        Some(deb) => deb,
        None => fs::read_to_string(request_path(&paths))
            .map_err(|error| format!("read upgrade request: {error}"))?
            .trim()
            .to_string(),
    };
    if deb.is_empty() {
        return Err("no deb to install (empty request)".to_string());
    }
    let mut host = SystemHost::new(paths);
    run_upgrade(&mut host, &deb, upgrade_timeout())
}

/// The real host: systemctl/dpkg/tar over the resolved runtime paths. ON-BOX
/// scaffolding — validated in STEP 6, not unit-tested. Command lines are kept
/// explicit here so tuning them on the box is localized.
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

    /// Run a privileged command via `sudo -n`. The oneshot upgrade unit runs as
    /// the santi user (so runtime files it writes stay santi-owned), and santi
    /// has passwordless sudo — so systemctl/dpkg go through sudo. `-n` never
    /// prompts: if sudo would need a password it fails fast + visibly.
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
        // systemd waits up to the unit's TimeoutStopSec (STEP 5 sets it ≥ the
        // service's shutdown grace), during which santi drains its in-flight turn.
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
        // Crisp gate: did the process come up + is the store coherent? Poll the
        // read-only doctor (schema migrated + memory readable) within the budget.
        let deadline = Instant::now() + upgrade_timeout();
        let healthy = loop {
            match crate::ops::doctor_at(&self.paths) {
                Ok(report) if report.ok => break true,
                _ if Instant::now() >= deadline => break false,
                _ => thread::sleep(Duration::from_millis(500)),
            }
        };
        // Stop again so the FINAL start (after seeding) is uniform.
        self.systemctl("stop")?;
        Ok(healthy)
    }

    fn rollback(&mut self) -> Result<(), String> {
        // Restore the runtime snapshot. The binary reinstall of the previous
        // version is wired on-box in STEP 6 (via SANTI_PREVIOUS_DEB); absent it,
        // restoring the runtime + leaving the binary is the best we can do here.
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
        if let Ok(prev) = env::var("SANTI_PREVIOUS_DEB").map(|v| v.trim().to_string())
            && !prev.is_empty()
        {
            self.install(&prev)?;
        }
        Ok(())
    }

    fn seed(&mut self, text: &str) -> Result<(), String> {
        let strand = env::var("SANTI_STRAND_ID")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or("no SANTI_STRAND_ID configured for the come-look record")?;
        let report = crate::ops::inbox_seed_at(&self.paths, &strand, text)?;
        if report.accepted {
            Ok(())
        } else {
            Err(report.reason.unwrap_or_else(|| "seed rejected".to_string()))
        }
    }

    fn start(&mut self) -> Result<(), String> {
        self.systemctl("start")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeHost {
        calls: Vec<String>,
        install_result: Result<(), String>,
        probe_result: Result<bool, String>,
        seed_result: Result<(), String>,
        seeded_text: Option<String>,
    }

    impl Default for FakeHost {
        fn default() -> Self {
            // Healthy defaults; each test overrides the field it exercises.
            Self {
                calls: Vec::new(),
                install_result: Ok(()),
                probe_result: Ok(true),
                seed_result: Ok(()),
                seeded_text: None,
            }
        }
    }

    impl UpgradeHost for FakeHost {
        fn graceful_stop(&mut self, _grace: Duration) -> Result<(), String> {
            self.calls.push("graceful_stop".into());
            Ok(())
        }
        fn snapshot(&mut self) -> Result<(), String> {
            self.calls.push("snapshot".into());
            Ok(())
        }
        fn install(&mut self, _deb: &str) -> Result<(), String> {
            self.calls.push("install".into());
            self.install_result.clone()
        }
        fn trial_probe(&mut self) -> Result<bool, String> {
            self.calls.push("trial_probe".into());
            self.probe_result.clone()
        }
        fn rollback(&mut self) -> Result<(), String> {
            self.calls.push("rollback".into());
            Ok(())
        }
        fn seed(&mut self, text: &str) -> Result<(), String> {
            self.calls.push("seed".into());
            self.seeded_text = Some(text.to_string());
            self.seed_result.clone()
        }
        fn start(&mut self) -> Result<(), String> {
            self.calls.push("start".into());
            Ok(())
        }
    }

    fn run(host: &mut FakeHost) -> UpgradeReport {
        run_upgrade(host, "santi_beta.deb", Duration::from_secs(1)).expect("run")
    }

    #[test]
    fn success_path_seeds_come_look_and_never_rolls_back() {
        let mut host = FakeHost {
            install_result: Ok(()),
            probe_result: Ok(true),
            seed_result: Ok(()),
            ..Default::default()
        };
        let report = run(&mut host);
        assert_eq!(
            host.calls,
            [
                "graceful_stop",
                "snapshot",
                "install",
                "trial_probe",
                "seed",
                "start"
            ]
        );
        assert_eq!(report.outcome, Outcome::Upgraded);
        assert!(report.seeded);
        // Seed happens BEFORE the final start (guaranteed drain on boot).
        let seed_i = host.calls.iter().position(|c| c == "seed").unwrap();
        let start_i = host.calls.iter().position(|c| c == "start").unwrap();
        assert!(seed_i < start_i);
        assert!(host.seeded_text.unwrap().contains("came back up"));
    }

    #[test]
    fn install_failure_rolls_back_and_seeds_the_reason() {
        let mut host = FakeHost {
            install_result: Err("bad package signature".into()),
            probe_result: Ok(true), // never reached
            seed_result: Ok(()),
            ..Default::default()
        };
        let report = run(&mut host);
        // No trial_probe (install never succeeded); rollback then seed then start.
        assert_eq!(
            host.calls,
            [
                "graceful_stop",
                "snapshot",
                "install",
                "rollback",
                "seed",
                "start"
            ]
        );
        assert_eq!(
            report.outcome,
            Outcome::RolledBack(RollbackCause::InstallFailed("bad package signature".into()))
        );
        let seeded = host.seeded_text.unwrap();
        assert!(seeded.contains("FAILED while installing"));
        assert!(seeded.contains("bad package signature"));
    }

    #[test]
    fn unhealthy_new_version_rolls_back() {
        let mut host = FakeHost {
            install_result: Ok(()),
            probe_result: Ok(false),
            seed_result: Ok(()),
            ..Default::default()
        };
        let report = run(&mut host);
        assert_eq!(
            host.calls,
            [
                "graceful_stop",
                "snapshot",
                "install",
                "trial_probe",
                "rollback",
                "seed",
                "start"
            ]
        );
        assert_eq!(
            report.outcome,
            Outcome::RolledBack(RollbackCause::DidNotComeUp)
        );
        assert!(host.seeded_text.unwrap().contains("did not come up"));
    }

    #[test]
    fn a_probe_error_is_treated_as_unhealthy() {
        let mut host = FakeHost {
            install_result: Ok(()),
            probe_result: Err("probe timed out".into()),
            seed_result: Ok(()),
            ..Default::default()
        };
        let report = run(&mut host);
        assert_eq!(
            report.outcome,
            Outcome::RolledBack(RollbackCause::DidNotComeUp)
        );
        assert!(host.calls.contains(&"rollback".to_string()));
    }

    #[test]
    fn seed_failure_does_not_abort_the_upgrade() {
        let mut host = FakeHost {
            install_result: Ok(()),
            probe_result: Ok(true),
            seed_result: Err("no self-strand configured".into()),
            ..Default::default()
        };
        let report = run(&mut host);
        assert_eq!(report.outcome, Outcome::Upgraded);
        assert!(!report.seeded, "seed reported not durably enqueued");
        // The final start still happens.
        assert_eq!(host.calls.last().unwrap(), "start");
    }
}
