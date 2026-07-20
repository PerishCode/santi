use super::*;

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
fn retention_failure_rolls_back() {
    let mut host = FakeHost {
        retain_result: Err("manifest sync failed".into()),
        ..Default::default()
    };
    let report = run(&mut host);
    assert_eq!(
        report.outcome,
        Outcome::RolledBack(RollbackCause::ArtifactRetentionFailed(
            "manifest sync failed".into()
        ))
    );
    assert_eq!(
        host.calls,
        [
            "graceful_stop",
            "snapshot",
            "install",
            "trial_probe",
            "retain_candidate",
            "rollback",
            "finalize",
            "start"
        ]
    );
    let UpgradeTerminal::RolledBack { failure } = &host.finalizations[0].terminal else {
        panic!("expected rolled-back terminal");
    };
    assert_eq!(failure.stage.operation(), "upgrade.retain_artifact");
    assert_eq!(failure.recovery, RecoveryStatus::PreviousVersionRestored);
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
