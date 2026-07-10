use std::{path::Path, time::Duration};

use santi_api::{
    config::RuntimePaths,
    upgrade::{
        Outcome, RollbackCause, SeedOutcome, UpgradeHost, UpgradeReport, run_upgrade,
        seed_come_look_at,
    },
};

struct FakeHost {
    calls: Vec<String>,
    install_result: Result<(), String>,
    probe_result: Result<bool, String>,
    seed_result: Result<SeedOutcome, String>,
    seeded_text: Option<String>,
}

impl Default for FakeHost {
    fn default() -> Self {
        Self {
            calls: Vec::new(),
            install_result: Ok(()),
            probe_result: Ok(true),
            seed_result: Ok(fake_seed()),
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

    fn seed(&mut self, text: &str) -> Result<SeedOutcome, String> {
        self.calls.push("seed".into());
        self.seeded_text = Some(text.to_string());
        self.seed_result.clone()
    }

    fn start(&mut self) -> Result<(), String> {
        self.calls.push("start".into());
        Ok(())
    }
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
            "seed",
            "start"
        ]
    );
    assert_eq!(report.outcome, Outcome::Upgraded);
    assert!(report.seeded);
    assert!(host.seeded_text.unwrap().contains("came back up"));
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
            "seed",
            "start"
        ]
    );
    assert!(host.seeded_text.unwrap().contains("bad package signature"));
}

#[test]
fn unhealthy_rolls_back() {
    let mut host = FakeHost {
        probe_result: Ok(false),
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
fn probe_error_rolls_back() {
    let mut host = FakeHost {
        probe_result: Err("probe timed out".into()),
        ..Default::default()
    };
    let report = run(&mut host);
    assert_eq!(
        report.outcome,
        Outcome::RolledBack(RollbackCause::DidNotComeUp)
    );
}

#[test]
fn seed_failure_is_reported() {
    let mut host = FakeHost {
        seed_result: Err("no self-strand configured".into()),
        ..Default::default()
    };
    let report = run(&mut host);
    assert_eq!(report.outcome, Outcome::Upgraded);
    assert!(!report.seeded);
    assert_eq!(
        report.warnings,
        vec!["come-look seed failed: no self-strand configured".to_string()]
    );
    assert_eq!(host.calls.last().map(String::as_str), Some("start"));
}

#[test]
fn stable_label_seeds() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    santi_core::SantiStore::open(&paths.database_path).expect("open");

    let outcome = seed_come_look_at(
        &paths,
        santi_core::DEFAULT_SOUL_ID,
        Some("ss_stale"),
        "come look",
    )
    .expect("seed via stable label");
    assert!(outcome.warnings.is_empty());

    let store = santi_core::SantiStore::open(&paths.database_path).expect("reopen");
    let strand = store.strand(&outcome.strand_id).unwrap().expect("strand");
    assert_eq!(
        strand.external_label.as_deref(),
        Some("soul:soul_default:ops")
    );
    let started = store
        .try_start_turn(&outcome.strand_id, "strand_send", None)
        .unwrap()
        .expect("turn starts");
    assert_eq!(started.drained_messages[0].content_text, "come look");
}

fn paths_under(root: &Path) -> RuntimePaths {
    RuntimePaths {
        database_path: root.join("runtime").join("db"),
        runtime_root: root.join("runtime"),
        execution_root: root.join("execution"),
    }
}
