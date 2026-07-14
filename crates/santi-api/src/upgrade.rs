mod artifacts;
mod finalize;
mod system;

pub use finalize::{finalize, finalize_at};
pub use system::{launch, run};

use std::env;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use finalize::{FINALIZE_PROTOCOL_VERSION, persistence_error};

const HANDOVER_BUDGET_PROFILE: &str = "upgrade_handover_audit_v1";
const HANDOVER_MAX_PROVIDER_ROUNDS: usize = 12;
const HANDOVER_MAX_TOOL_CALLS: usize = 16;
const HANDOVER_MAX_TOOL_OUTPUT_BYTES: usize = 40 * 1024;
const HANDOVER_MAX_SHELL_OUTPUT_BYTES: usize = 16 * 1024;

pub fn register_attempt_handover_budgets(
    service: &santi_core::service::Service,
) -> Result<usize, String> {
    let mut registered = 0;
    for strand in service.list_strands()? {
        let expected_prefix = format!("soul:{}:ops:upgrade:", strand.soul_id);
        let is_attempt_handover = strand
            .external_label
            .as_deref()
            .and_then(|label| label.strip_prefix(&expected_prefix))
            .is_some_and(|attempt_id| !attempt_id.is_empty());
        if !is_attempt_handover {
            continue;
        }
        service.set_strand_execution_budget(
            &strand.id,
            santi_core::Execution {
                profile: HANDOVER_BUDGET_PROFILE.to_string(),
                max_provider_rounds: HANDOVER_MAX_PROVIDER_ROUNDS,
                max_tool_calls: HANDOVER_MAX_TOOL_CALLS,
                max_tool_output_bytes: HANDOVER_MAX_TOOL_OUTPUT_BYTES,
                max_shell_output_bytes: HANDOVER_MAX_SHELL_OUTPUT_BYTES,
            },
        )?;
        registered += 1;
    }
    Ok(registered)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "cause", content = "detail")]
pub enum RollbackCause {
    InstallFailed(String),
    DidNotComeUp(String),
    ArtifactRetentionFailed(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum Outcome {
    Upgraded { readiness: UpgradeReadiness },
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
    RetainArtifact,
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
            Self::RetainArtifact => "upgrade.retain_artifact",
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpgradeReport {
    pub attempt_id: String,
    pub outcome: Outcome,
    pub record: String,
    pub seeded: bool,
    pub seeded_strand_id: Option<String>,
    pub errors: Vec<santi_core::SantiError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SeedOutcome {
    pub strand_id: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpgradeStarted {
    pub attempt_id: String,
    pub status: &'static str,
    pub timeout_secs: u64,
    pub log_hint: String,
    pub candidate_version: String,
    pub candidate_sha256: String,
    pub previous_version: String,
    pub previous_sha256: String,
}

pub trait UpgradeHost {
    fn graceful_stop(&mut self, grace: Duration) -> Result<(), String>;
    fn snapshot(&mut self) -> Result<(), String>;
    fn install(&mut self, deb: &str) -> Result<(), String>;
    fn trial_probe(&mut self) -> Result<UpgradeReadiness, String>;
    fn retain_candidate(&mut self) -> Result<(), String>;
    fn rollback(&mut self) -> Result<(), String>;
    fn finalize(
        &mut self,
        request: &UpgradeFinalizeRequest,
    ) -> Result<UpgradeFinalizeReport, String>;
    fn start(&mut self) -> Result<(), String>;
}

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

    let outcome = match host.install(deb) {
        Err(error) => Outcome::RolledBack(RollbackCause::InstallFailed(error)),
        Ok(()) => match host.trial_probe() {
            Ok(readiness) => match host.retain_candidate() {
                Ok(()) => Outcome::Upgraded { readiness },
                Err(error) => Outcome::RolledBack(RollbackCause::ArtifactRetentionFailed(error)),
            },
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

pub fn compose_record(attempt_id: &str) -> String {
    format!(
        "A santi self-upgrade attempt (`{attempt_id}`) reached handover. Perform a bounded \
         current-state audit, then record a concise assessment. Check doctor/service readiness \
         and only the incidents relevant to this attempt. Do not enumerate artifact directories, \
         dump database schemas or full incident histories, or print unbounded journals."
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
                    RollbackCause::ArtifactRetentionFailed(detail) => UpgradeFailure {
                        stage: UpgradeStage::RetainArtifact,
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
