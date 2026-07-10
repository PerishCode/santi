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
//!   It orchestrates the sequence below. The binary selected as FINAL records
//!   canonical incidents and seeds a neutral wake before the final start.
//!
//! Sequence (`run_upgrade`): graceful-stop → snapshot → dpkg → resolve (install +
//! a trial start/health probe) → auto-rollback on a CRISP failure → finalize
//! terminal truth + wake (offline, with the FINAL binary) → start FINAL.
//!
//! The side effects live behind [`UpgradeHost`] so the orchestration LOGIC here
//! is unit-tested with a fake; the real systemctl/dpkg shell is
//! validated on a Debian box (PHASE-07 STEP 6), not in CI.

mod finalize;
mod system;

pub use finalize::{finalize, finalize_at, seed_come_look_at};
pub use system::{launch, run};

use std::env;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use finalize::{FINALIZE_PROTOCOL_VERSION, persistence_error};

/// Why the runner rolled back to the previous version. The detail is durable
/// operator truth in the error incident; it is never projected into come-look.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "cause", content = "detail")]
pub enum RollbackCause {
    InstallFailed(String),
    DidNotComeUp(String),
}

/// The resolved outcome of an upgrade attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum Outcome {
    /// The new version installed and came up; it is the final version. A
    /// degraded runtime is still live and must not be rolled back implicitly.
    Upgraded { readiness: UpgradeReadiness },
    /// A crisp pre-commit failure → restored to the previous version.
    RolledBack(RollbackCause),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpgradeReadiness {
    #[default]
    Ready,
    Degraded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpgradeStage {
    Launch,
    ResolveRequest,
    GracefulStop,
    Snapshot,
    Install,
    TrialProbe,
    Rollback,
    FinalStart,
}

impl UpgradeStage {
    pub fn operation(self) -> &'static str {
        match self {
            Self::Launch => "upgrade.launch",
            Self::ResolveRequest => "upgrade.resolve_request",
            Self::GracefulStop => "upgrade.graceful_stop",
            Self::Snapshot => "upgrade.snapshot",
            Self::Install => "upgrade.install",
            Self::TrialProbe => "upgrade.trial_probe",
            Self::Rollback => "upgrade.rollback",
            Self::FinalStart => "upgrade.final_start",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryStatus {
    Unknown,
    PreviousVersionRestarted,
    PreviousVersionRestored,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpgradeFailure {
    pub stage: UpgradeStage,
    pub detail: String,
    pub recovery: RecoveryStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "terminal")]
pub enum UpgradeTerminal {
    Upgraded,
    RolledBack { failure: UpgradeFailure },
    Failed { failure: UpgradeFailure },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpgradeFinalizeRequest {
    pub protocol_version: u32,
    pub attempt_id: String,
    pub deb: String,
    pub terminal: UpgradeTerminal,
    #[serde(default)]
    pub readiness: UpgradeReadiness,
    pub wake: bool,
    pub soul_id: String,
    pub configured_strand_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpgradeFinalizeReport {
    pub errors: Vec<santi_core::SantiError>,
    pub record: Option<String>,
    pub seeded: bool,
    pub seeded_strand_id: Option<String>,
}

impl Outcome {
    fn is_rollback(&self) -> bool {
        matches!(self, Outcome::RolledBack(_))
    }
}

/// What the runner did, for the log + the launcher's after-the-fact inspection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpgradeReport {
    pub attempt_id: String,
    pub outcome: Outcome,
    /// Neutral wake occurrence. Error detail lives only in `errors`/incidents.
    pub record: String,
    pub seeded: bool,
    /// The concrete strand that received the record. It is a materialized room;
    /// the durable addressing anchor is a stable label.
    pub seeded_strand_id: Option<String>,
    /// Canonical execution and/or handover failures observed in this attempt.
    pub errors: Vec<santi_core::SantiError>,
}

/// The successful result of seeding a come-look record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SeedOutcome {
    pub strand_id: String,
    pub warnings: Vec<String>,
}

/// The launcher's fast return: "started listening, max timeout, log location".
#[derive(Debug, Clone, Serialize)]
pub struct UpgradeStarted {
    pub attempt_id: String,
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
    /// then stop it again so the final start is uniform. Degraded means the
    /// process is live but canonical incidents still require intervention.
    fn trial_probe(&mut self) -> Result<UpgradeReadiness, String>;
    /// Restore the snapshot + reinstall the previous version (the final = OLD).
    fn rollback(&mut self) -> Result<(), String>;
    /// Ask the binary currently installed on disk (the FINAL version) to record
    /// terminal incidents and optionally seed the neutral wake occurrence.
    fn finalize(
        &mut self,
        request: &UpgradeFinalizeRequest,
    ) -> Result<UpgradeFinalizeReport, String>;
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
) -> Result<UpgradeReport, Box<santi_core::SantiError>> {
    let attempt_id = format!("upgrade_{}", Uuid::new_v4().simple());
    run_upgrade_attempt(host, deb, grace, attempt_id)
}

pub(super) fn run_upgrade_attempt<H: UpgradeHost>(
    host: &mut H,
    deb: &str,
    grace: Duration,
    attempt_id: String,
) -> Result<UpgradeReport, Box<santi_core::SantiError>> {
    if let Err(detail) = host.graceful_stop(grace) {
        let failure = UpgradeFailure {
            stage: UpgradeStage::GracefulStop,
            detail,
            recovery: RecoveryStatus::Unknown,
        };
        return Err(record_fatal(host, &attempt_id, deb, failure));
    }
    if let Err(detail) = host.snapshot() {
        let (detail, recovery) = match host.start() {
            Ok(()) => (detail, RecoveryStatus::PreviousVersionRestarted),
            Err(start_error) => (
                format!("{detail}; previous version restart also failed: {start_error}"),
                RecoveryStatus::Failed,
            ),
        };
        let failure = UpgradeFailure {
            stage: UpgradeStage::Snapshot,
            detail,
            recovery,
        };
        return Err(record_fatal(host, &attempt_id, deb, failure));
    }

    // Resolve the final version. A crisp failure (install error, or the new
    // version does not come up) routes to rollback; anything else is Upgraded.
    let outcome = match host.install(deb) {
        Err(error) => Outcome::RolledBack(RollbackCause::InstallFailed(error)),
        Ok(()) => match host.trial_probe() {
            Ok(readiness) => Outcome::Upgraded { readiness },
            // A probe that could not prove the process came up routes to a
            // conservative rollback. Explicit degraded readiness does not.
            Err(error) => Outcome::RolledBack(RollbackCause::DidNotComeUp(error)),
        },
    };

    if outcome.is_rollback()
        && let Err(detail) = host.rollback()
    {
        let failure = UpgradeFailure {
            stage: UpgradeStage::Rollback,
            detail,
            recovery: RecoveryStatus::Failed,
        };
        return Err(record_fatal(host, &attempt_id, deb, failure));
    }

    let (terminal, readiness) = terminal_from_outcome(&outcome);
    let finalize_result = host.finalize(&UpgradeFinalizeRequest {
        protocol_version: FINALIZE_PROTOCOL_VERSION,
        attempt_id: attempt_id.clone(),
        deb: deb.to_string(),
        terminal,
        readiness,
        wake: true,
        soul_id: santi_core::DEFAULT_SOUL_ID.to_string(),
        configured_strand_id: None,
    });

    if let Err(detail) = host.start() {
        let failure = UpgradeFailure {
            stage: UpgradeStage::FinalStart,
            detail,
            recovery: RecoveryStatus::Failed,
        };
        return Err(record_fatal(host, &attempt_id, deb, failure));
    }

    let finalized = finalize_result.map_err(|error| {
        Box::new(persistence_error(
            &attempt_id,
            deb,
            "upgrade.finalize",
            error,
        ))
    })?;
    let record = finalized.record.ok_or_else(|| {
        Box::new(persistence_error(
            &attempt_id,
            deb,
            "upgrade.finalize",
            "finalizer omitted the requested wake record",
        ))
    })?;

    Ok(UpgradeReport {
        attempt_id,
        outcome,
        record,
        seeded: finalized.seeded,
        seeded_strand_id: finalized.seeded_strand_id,
        errors: finalized.errors,
    })
}

/// A wake occurrence, not an error projection. The model independently checks
/// the runtime and may audit canonical incidents/operator logs when needed.
pub fn compose_record(attempt_id: &str) -> String {
    format!(
        "A santi self-upgrade attempt (`{attempt_id}`) reached handover. Inspect the current \
         runtime state and audit runtime error incidents before continuing."
    )
}

fn terminal_from_outcome(outcome: &Outcome) -> (UpgradeTerminal, UpgradeReadiness) {
    match outcome {
        Outcome::Upgraded { readiness } => (UpgradeTerminal::Upgraded, *readiness),
        Outcome::RolledBack(cause) => (
            UpgradeTerminal::RolledBack {
                failure: match cause {
                    RollbackCause::InstallFailed(detail) => UpgradeFailure {
                        stage: UpgradeStage::Install,
                        detail: detail.clone(),
                        recovery: RecoveryStatus::PreviousVersionRestored,
                    },
                    RollbackCause::DidNotComeUp(detail) => UpgradeFailure {
                        stage: UpgradeStage::TrialProbe,
                        detail: detail.clone(),
                        recovery: RecoveryStatus::PreviousVersionRestored,
                    },
                },
            },
            UpgradeReadiness::Ready,
        ),
    }
}

fn record_fatal<H: UpgradeHost>(
    host: &mut H,
    attempt_id: &str,
    deb: &str,
    failure: UpgradeFailure,
) -> Box<santi_core::SantiError> {
    let operation = failure.stage.operation();
    let request = UpgradeFinalizeRequest {
        protocol_version: FINALIZE_PROTOCOL_VERSION,
        attempt_id: attempt_id.to_string(),
        deb: deb.to_string(),
        terminal: UpgradeTerminal::Failed { failure },
        readiness: UpgradeReadiness::Ready,
        wake: false,
        soul_id: santi_core::DEFAULT_SOUL_ID.to_string(),
        configured_strand_id: None,
    };
    match host.finalize(&request) {
        Ok(report) => Box::new(
            report
                .errors
                .into_iter()
                .find(|error| error.code == santi_core::catalog::UPGRADE_FAILED.code)
                .unwrap_or_else(|| {
                    persistence_error(
                        attempt_id,
                        deb,
                        operation,
                        "finalizer returned no upgrade failure incident",
                    )
                }),
        ),
        Err(error) => Box::new(persistence_error(attempt_id, deb, operation, error)),
    }
}

pub fn upgrade_timeout() -> Duration {
    let secs = env::var("SANTI_UPGRADE_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(600);
    Duration::from_secs(secs)
}
