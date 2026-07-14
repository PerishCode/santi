use std::fs;
use std::path::Path;

use tempfile::tempdir;

#[path = "../src/upgrade/artifacts.rs"]
mod artifacts;

use artifacts::{Dpkg, Identity, Probe, Store};

struct FakeProbe {
    installed_version: &'static str,
}

impl Probe for FakeProbe {
    fn inspect_deb(&self, path: &Path) -> Result<Identity, String> {
        let content = fs::read_to_string(path).map_err(|error| error.to_string())?;
        let mut fields = content.splitn(3, '|');
        Ok(Identity {
            package: fields.next().unwrap_or_default().to_string(),
            version: fields.next().unwrap_or_default().to_string(),
        })
    }

    fn installed(&self) -> Result<Identity, String> {
        Ok(Identity {
            package: "santi".to_string(),
            version: self.installed_version.to_string(),
        })
    }
}

fn package(path: &Path, version: &str, payload: &str) {
    fs::write(path, format!("santi|{version}|{payload}")).unwrap();
}

#[test]
fn dpkg_probe_fails_closed() {
    let probe = Dpkg;
    let error = probe
        .inspect_deb(Path::new("definitely-missing-santi-package.deb"))
        .unwrap_err();
    assert!(!error.is_empty());
}

#[test]
fn bootstrap_reuses_and_promotes() {
    let temp = tempdir().unwrap();
    let store = Store::new(temp.path());
    let previous = temp.path().join("previous.deb");
    let candidate = temp.path().join("candidate.deb");
    package(&previous, "1.0.0", "previous");
    package(&candidate, "2.0.0", "candidate");

    let old_probe = FakeProbe {
        installed_version: "1.0.0",
    };
    let previous_artifact = store.resolve_previous(Some(&previous), &old_probe).unwrap();
    assert_eq!(
        store.resolve_previous(None, &old_probe).unwrap(),
        previous_artifact
    );

    let candidate_artifact = store.stage(&candidate, &old_probe).unwrap();
    let new_probe = FakeProbe {
        installed_version: "2.0.0",
    };
    store
        .commit_installed(&candidate_artifact, &new_probe)
        .unwrap();
    assert_eq!(
        store.load_installed(&new_probe).unwrap(),
        Some(candidate_artifact)
    );
}

#[test]
fn rejects_identity_and_tampering() {
    let temp = tempdir().unwrap();
    let store = Store::new(temp.path());
    let wrong = temp.path().join("wrong.deb");
    package(&wrong, "0.9.0", "wrong");
    let probe = FakeProbe {
        installed_version: "1.0.0",
    };
    assert!(
        store
            .resolve_previous(Some(&wrong), &probe)
            .unwrap_err()
            .contains("does not match installed package")
    );

    let current = temp.path().join("current.deb");
    package(&current, "1.0.0", "current");
    let artifact = store.resolve_previous(Some(&current), &probe).unwrap();
    let conflicting = temp.path().join("conflicting.deb");
    package(&conflicting, "1.0.0", "different bytes");
    assert!(
        store
            .resolve_previous(Some(&conflicting), &probe)
            .unwrap_err()
            .contains("differs from durable installed artifact")
    );

    fs::write(store.path_for(&artifact).unwrap(), "santi|1.0.0|tampered").unwrap();
    assert!(
        store
            .verify(&artifact, &probe)
            .unwrap_err()
            .contains("mismatch")
    );
}

#[test]
fn interruption_preserves_manifest() {
    let temp = tempdir().unwrap();
    let store = Store::new(temp.path());
    let previous = temp.path().join("previous.deb");
    let candidate = temp.path().join("candidate.deb");
    package(&previous, "1.0.0", "previous");
    package(&candidate, "2.0.0", "candidate");
    let probe = FakeProbe {
        installed_version: "1.0.0",
    };
    let previous_artifact = store.resolve_previous(Some(&previous), &probe).unwrap();
    store.stage(&candidate, &probe).unwrap();
    fs::write(
        temp.path()
            .join("upgrade/.installed-package.json.interrupted.tmp"),
        "partial",
    )
    .unwrap();
    assert_eq!(
        store.load_installed(&probe).unwrap(),
        Some(previous_artifact)
    );
}

#[test]
fn prune_cleans_orphans() {
    let temp = tempdir().unwrap();
    let store = Store::new(temp.path());
    let probe = FakeProbe {
        installed_version: "1.0.0",
    };
    let first = temp.path().join("first.deb");
    let second = temp.path().join("second.deb");
    let old = temp.path().join("old.deb");
    package(&first, "1.0.0", "first");
    package(&second, "2.0.0", "second");
    package(&old, "0.9.0", "old");
    let first = store.stage(&first, &probe).unwrap();
    let second = store.stage(&second, &probe).unwrap();
    let old = store.stage(&old, &probe).unwrap();
    let interrupted = temp.path().join("upgrade/packages/.stage-interrupted.tmp");
    fs::write(&interrupted, "partial").unwrap();

    store.prune(&[&first, &second]).unwrap();
    assert!(store.path_for(&first).unwrap().is_file());
    assert!(store.path_for(&second).unwrap().is_file());
    assert!(!store.path_for(&old).unwrap().exists());
    assert!(!interrupted.exists());
}
