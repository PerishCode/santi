use super::*;

pub fn run_upgrade<H: UpgradeHost>(
    host: &mut H,
    deb: &str,
    grace: Duration,
) -> Result<UpgradeReport, Box<santi_core::Fault>> {
    let attempt_id = format!("upgrade_{}", Uuid::new_v4().simple());
    run_upgrade_attempt(host, deb, grace, attempt_id)
}

pub(super) fn run_upgrade_attempt<H: UpgradeHost>(
    host: &mut H,
    deb: &str,
    grace: Duration,
    attempt_id: String,
) -> Result<UpgradeReport, Box<santi_core::Fault>> {
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
) -> Box<santi_core::Fault> {
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
    crate::runtime::held().upgrade_timeout
}
