use std::time::Duration;

use santi_api::upgrade::{
    Outcome, RecoveryStatus, RollbackCause, SeedOutcome, UpgradeFinalizeReport,
    UpgradeFinalizeRequest, UpgradeHost, UpgradeReadiness, UpgradeReport, UpgradeTerminal,
    compose_record, run_upgrade,
};
use santi_core::{Fault, catalog, engine};

struct FakeHost {
    calls: Vec<String>,
    install_result: Result<(), String>,
    probe_result: Result<UpgradeReadiness, String>,
    retain_result: Result<(), String>,
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
            retain_result: Ok(()),
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

    fn retain_candidate(&mut self) -> Result<(), String> {
        self.calls.push("retain_candidate".into());
        self.retain_result.clone()
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
            Ok(seed) if seed.warnings.is_empty() => (true, Some(seed.strand)),
            Ok(seed) => {
                errors.push(fake_error(
                    catalog::UPGRADE_HANDOVER_FAILED,
                    "upgrade.handover",
                    &seed.warnings.join("; "),
                ));
                (true, Some(seed.strand))
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

fn fake_error(descriptor: santi_core::Descriptor, operation: &str, detail: &str) -> Fault {
    engine()
        .open(
            None,
            santi_core::error::Draft {
                key: format!("{}:runtime:default", descriptor.code),
                descriptor,
                scope: santi_core::Scope::new("runtime", "default"),
                source: santi_core::Source::new("santi-api", operation),
                message: detail.to_string(),
                context: serde_json::json!({"detail": detail}),
            },
            "test",
        )
        .error
}

fn fake_seed() -> SeedOutcome {
    SeedOutcome {
        strand: "ss_seeded".to_string(),
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
            "retain_candidate",
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
    let seeded_text = host.seeded_text.unwrap();
    assert!(seeded_text.contains(&report.attempt_id));
    assert!(seeded_text.contains("bounded current-state audit"));
    assert!(seeded_text.contains("Do not enumerate artifact directories"));
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

mod rollback;
