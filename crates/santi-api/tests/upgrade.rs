use std::time::Duration;

use santi_api::upgrade::{
    Outcome, RecoveryStatus, RollbackCause, SeedOutcome, UpgradeFinalizeReport,
    UpgradeFinalizeRequest, UpgradeHost, UpgradeReadiness, UpgradeReport, UpgradeTerminal,
    compose_record, run_upgrade,
};
use santi_core::{ErrorScope, ErrorSource, IncidentDraft, SantiError, catalog, engine};

struct FakeHost {
    calls: Vec<String>,
    install_result: Result<(), String>,
    probe_result: Result<UpgradeReadiness, String>,
    seed_result: Result<SeedOutcome, String>,
    graceful_stop_result: Result<(), String>,
    snapshot_result: Result<(), String>,
    rollback_result: Result<(), String>,
    start_result: Result<(), String>,
    finalize_result: Result<(), String>,
    seeded_text: Option<String>,
    finalizations: Vec<UpgradeFinalizeRequest>,
}

impl Default for FakeHost {
    fn default() -> Self {
        Self {
            calls: Vec::new(),
            install_result: Ok(()),
            probe_result: Ok(UpgradeReadiness::Ready),
            seed_result: Ok(fake_seed()),
            graceful_stop_result: Ok(()),
            snapshot_result: Ok(()),
            rollback_result: Ok(()),
            start_result: Ok(()),
            finalize_result: Ok(()),
            seeded_text: None,
            finalizations: Vec::new(),
        }
    }
}

impl UpgradeHost for FakeHost {
    fn graceful_stop(&mut self, _grace: Duration) -> Result<(), String> {
        self.calls.push("graceful_stop".into());
        self.graceful_stop_result.clone()
    }

    fn snapshot(&mut self) -> Result<(), String> {
        self.calls.push("snapshot".into());
        self.snapshot_result.clone()
    }

    fn install(&mut self, _deb: &str) -> Result<(), String> {
        self.calls.push("install".into());
        self.install_result.clone()
    }

    fn trial_probe(&mut self) -> Result<UpgradeReadiness, String> {
        self.calls.push("trial_probe".into());
        self.probe_result.clone()
    }

    fn rollback(&mut self) -> Result<(), String> {
        self.calls.push("rollback".into());
        self.rollback_result.clone()
    }

    fn finalize(
        &mut self,
        request: &UpgradeFinalizeRequest,
    ) -> Result<UpgradeFinalizeReport, String> {
        self.calls.push("finalize".into());
        self.finalizations.push(request.clone());
        self.finalize_result.clone()?;

        let mut errors = match &request.terminal {
            UpgradeTerminal::Upgraded => Vec::new(),
            UpgradeTerminal::RolledBack { failure } | UpgradeTerminal::Failed { failure } => {
                vec![fake_error(
                    catalog::UPGRADE_FAILED,
                    failure.stage.operation(),
                    &failure.detail,
                )]
            }
        };
        if !request.wake {
            return Ok(UpgradeFinalizeReport {
                errors,
                record: None,
                seeded: false,
                seeded_strand_id: None,
            });
        }

        let record = compose_record(&request.attempt_id);
        self.seeded_text = Some(record.clone());
        let (seeded, seeded_strand_id) = match self.seed_result.clone() {
            Ok(seed) if seed.warnings.is_empty() => (true, Some(seed.strand_id)),
            Ok(seed) => {
                errors.push(fake_error(
                    catalog::UPGRADE_HANDOVER_FAILED,
                    "upgrade.handover",
                    &seed.warnings.join("; "),
                ));
                (true, Some(seed.strand_id))
            }
            Err(error) => {
                errors.push(fake_error(
                    catalog::UPGRADE_HANDOVER_FAILED,
                    "upgrade.handover",
                    &error,
                ));
                (false, None)
            }
        };
        Ok(UpgradeFinalizeReport {
            errors,
            record: Some(record),
            seeded,
            seeded_strand_id,
        })
    }

    fn start(&mut self) -> Result<(), String> {
        self.calls.push("start".into());
        self.start_result.clone()
    }
}

fn fake_error(
    descriptor: santi_core::ErrorDescriptor,
    operation: &str,
    detail: &str,
) -> SantiError {
    engine()
        .open_incident(
            None,
            IncidentDraft {
                incident_key: format!("{}:runtime:default", descriptor.code),
                descriptor,
                scope: ErrorScope::new("runtime", "default"),
                source: ErrorSource::new("santi-api", operation),
                message: detail.to_string(),
                context: serde_json::json!({"detail": detail}),
            },
            "test",
        )
        .error
}

fn fake_seed() -> SeedOutcome {
    SeedOutcome {
        strand_id: "ss_seeded".to_string(),
        warnings: Vec::new(),
    }
}

fn run(host: &mut FakeHost) -> UpgradeReport {
    run_upgrade(host, "santi_beta.deb", Duration::from_secs(1)).expect("run")
}

#[test]
fn success_orders_steps() {
    let mut host = FakeHost::default();
    let report = run(&mut host);
    assert_eq!(
        host.calls,
        [
            "graceful_stop",
            "snapshot",
            "install",
            "trial_probe",
            "finalize",
            "start"
        ]
    );
    assert_eq!(
        report.outcome,
        Outcome::Upgraded {
            readiness: UpgradeReadiness::Ready
        }
    );
    assert!(report.seeded);
    assert!(report.errors.is_empty());
    assert!(host.seeded_text.unwrap().contains(&report.attempt_id));
}

#[test]
fn degraded_skips_rollback() {
    let mut host = FakeHost {
        probe_result: Ok(UpgradeReadiness::Degraded),
        ..Default::default()
    };
    let report = run(&mut host);
    assert_eq!(
        report.outcome,
        Outcome::Upgraded {
            readiness: UpgradeReadiness::Degraded
        }
    );
    assert!(!host.calls.contains(&"rollback".to_string()));
    assert!(matches!(
        host.finalizations[0].terminal,
        UpgradeTerminal::Upgraded
    ));
    assert_eq!(host.finalizations[0].readiness, UpgradeReadiness::Degraded);
}

#[test]
fn install_failure_rolls_back() {
    let mut host = FakeHost {
        install_result: Err("bad package signature".into()),
        ..Default::default()
    };
    let report = run(&mut host);
    assert_eq!(
        report.outcome,
        Outcome::RolledBack(RollbackCause::InstallFailed("bad package signature".into()))
    );
    assert_eq!(
        host.calls,
        [
            "graceful_stop",
            "snapshot",
            "install",
            "rollback",
            "finalize",
            "start"
        ]
    );
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].code, "runtime.upgrade.failed");
    assert!(!host.seeded_text.unwrap().contains("bad package signature"));
    let UpgradeTerminal::RolledBack { failure } = &host.finalizations[0].terminal else {
        panic!("expected rolled-back terminal");
    };
    assert_eq!(failure.recovery, RecoveryStatus::PreviousVersionRestored);
}

#[test]
fn unavailable_rolls_back() {
    let mut host = FakeHost {
        probe_result: Err("health probe returned unavailable".into()),
        ..Default::default()
    };
    let report = run(&mut host);
    assert_eq!(
        report.outcome,
        Outcome::RolledBack(RollbackCause::DidNotComeUp(
            "health probe returned unavailable".into()
        ))
    );
    assert!(host.calls.contains(&"rollback".to_string()));
}

#[test]
fn probe_error_rolls_back() {
    let mut host = FakeHost {
        probe_result: Err("probe timed out".into()),
        ..Default::default()
    };
    let report = run(&mut host);
    assert_eq!(
        report.outcome,
        Outcome::RolledBack(RollbackCause::DidNotComeUp("probe timed out".into()))
    );
    assert_eq!(report.errors[0].context["detail"], "probe timed out");
}

#[test]
fn seed_failure_is_reported() {
    let mut host = FakeHost {
        seed_result: Err("no self-strand configured".into()),
        ..Default::default()
    };
    let report = run(&mut host);
    assert_eq!(
        report.outcome,
        Outcome::Upgraded {
            readiness: UpgradeReadiness::Ready
        }
    );
    assert!(!report.seeded);
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].code, "runtime.upgrade.handover_failed");
    assert_eq!(host.calls.last().map(String::as_str), Some("start"));
}

#[test]
fn snapshot_failure_recovers() {
    let mut host = FakeHost {
        snapshot_result: Err("snapshot disk full".into()),
        ..Default::default()
    };
    let error = run_upgrade(&mut host, "santi_beta.deb", Duration::from_secs(1))
        .expect_err("snapshot must fail");
    assert_eq!(error.code, "runtime.upgrade.failed");
    assert_eq!(
        host.calls,
        ["graceful_stop", "snapshot", "start", "finalize"]
    );
    let UpgradeTerminal::Failed { failure } = &host.finalizations[0].terminal else {
        panic!("expected failed terminal");
    };
    assert_eq!(
        failure.recovery,
        santi_api::upgrade::RecoveryStatus::PreviousVersionRestarted
    );
}

#[test]
fn rollback_failure_stays_stopped() {
    let mut host = FakeHost {
        install_result: Err("bad package".into()),
        rollback_result: Err("restore failed".into()),
        ..Default::default()
    };
    let error = run_upgrade(&mut host, "santi_beta.deb", Duration::from_secs(1))
        .expect_err("rollback must fail");
    assert_eq!(error.code, "runtime.upgrade.failed");
    assert_eq!(
        host.calls,
        [
            "graceful_stop",
            "snapshot",
            "install",
            "rollback",
            "finalize"
        ]
    );
}

#[test]
fn start_failure_is_recorded() {
    let mut host = FakeHost {
        start_result: Err("unit failed".into()),
        ..Default::default()
    };
    let error = run_upgrade(&mut host, "santi_beta.deb", Duration::from_secs(1))
        .expect_err("start must fail");
    assert_eq!(error.code, "runtime.upgrade.failed");
    assert_eq!(
        host.calls.iter().filter(|call| *call == "finalize").count(),
        2
    );
    assert!(matches!(
        host.finalizations[1].terminal,
        UpgradeTerminal::Failed { .. }
    ));
}

#[test]
fn finalizer_failure_is_loud() {
    let mut host = FakeHost {
        finalize_result: Err("store unavailable".into()),
        ..Default::default()
    };
    let error = run_upgrade(&mut host, "santi_beta.deb", Duration::from_secs(1))
        .expect_err("finalizer must fail");
    assert_eq!(error.code, "runtime.error_engine.persistence_failed");
    assert_eq!(host.calls.last().map(String::as_str), Some("start"));
}
