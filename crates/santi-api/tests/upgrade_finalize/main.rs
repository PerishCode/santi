use std::{path::Path, sync::Arc};

use async_trait::async_trait;
use futures_util::stream;

use santi_api::{
    config::RuntimePaths,
    upgrade::{
        RecoveryStatus, UpgradeFailure, UpgradeFinalizeRequest, UpgradeReadiness, UpgradeStage,
        UpgradeTerminal, finalize_at, register_attempt_handover_budgets,
    },
};
use santi_core::SantiStore;
use santi_core::service::{self, Service};
use santi_provider::{
    ProviderClient, ProviderEvent, ProviderMetadata, ProviderRequest, ProviderStream,
};

#[derive(Clone)]
struct NoopProvider;

#[async_trait]
impl ProviderClient for NoopProvider {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            provider: Arc::from("noop"),
            model: "noop".to_string(),
            context_budget: None,
        }
    }

    async fn stream_response(&self, _request: ProviderRequest) -> Result<ProviderStream, String> {
        Ok(Box::pin(stream::iter(vec![Ok(ProviderEvent::Completed {
            provider_response_id: Some("noop".to_string()),
        })])))
    }
}

#[test]
fn old_request_defaults() {
    let request: UpgradeFinalizeRequest = serde_json::from_value(serde_json::json!({
        "protocol_version": 1,
        "attempt_id": "upgrade_old_runner",
        "deb": "/tmp/santi.deb",
        "terminal": {"terminal": "upgraded"},
        "wake": true,
        "soul_id": "soul_default",
        "configured_strand_id": null,
    }))
    .expect("decode old runner request");
    assert_eq!(request.readiness, UpgradeReadiness::Ready);
    assert!(matches!(request.terminal, UpgradeTerminal::Upgraded));
}

#[test]
fn attempt_labels_isolate_rooms() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    SantiStore::open(&paths.database_path).expect("open");

    let outcome = paths
        .seed_attempt_handover(
            santi_core::DEFAULT_SOUL_ID,
            "upgrade_one",
            Some("ss_stale"),
            "come look",
        )
        .expect("seed via attempt label");
    assert!(outcome.warnings.is_empty());
    let retry = paths
        .seed_attempt_handover(
            santi_core::DEFAULT_SOUL_ID,
            "upgrade_one",
            Some("ss_stale"),
            "come look again",
        )
        .expect("repeat seed via attempt label");
    assert_eq!(retry.strand_id, outcome.strand_id);
    let other = paths
        .seed_attempt_handover(
            santi_core::DEFAULT_SOUL_ID,
            "upgrade_two",
            Some("ss_stale"),
            "other attempt",
        )
        .expect("seed other attempt");
    assert_ne!(other.strand_id, outcome.strand_id);

    let store = SantiStore::open(&paths.database_path).expect("reopen");
    let strand = store.strand(&outcome.strand_id).unwrap().expect("strand");
    assert_eq!(
        strand.external_label.as_deref(),
        Some("soul:soul_default:ops:upgrade:upgrade_one")
    );
    let other_strand = store
        .strand(&other.strand_id)
        .unwrap()
        .expect("other strand");
    assert_eq!(
        other_strand.external_label.as_deref(),
        Some("soul:soul_default:ops:upgrade:upgrade_two")
    );
    let started = store
        .try_start_turn(&outcome.strand_id, "strand_send", None)
        .unwrap()
        .expect("turn starts");
    assert_eq!(started.drained_messages[0].content_text, "come look");
    assert_eq!(started.drained_messages[1].content_text, "come look again");
}

#[test]
fn stable_helper_preserves_label() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    SantiStore::open(&paths.database_path).expect("open");

    let outcome = paths
        .seed_come_look(santi_core::DEFAULT_SOUL_ID, None, "stable wake")
        .expect("seed stable label");
    let store = SantiStore::open(&paths.database_path).expect("reopen");
    let strand = store.strand(&outcome.strand_id).unwrap().expect("strand");
    assert_eq!(
        strand.external_label.as_deref(),
        Some("soul:soul_default:ops")
    );
}

#[test]
fn registers_attempt_rooms() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    paths
        .seed_attempt_handover(
            santi_core::DEFAULT_SOUL_ID,
            "upgrade_budgeted",
            None,
            "bounded wake",
        )
        .expect("seed attempt room");
    paths
        .seed_come_look(santi_core::DEFAULT_SOUL_ID, None, "stable wake")
        .expect("seed stable room");
    SantiStore::open(&paths.database_path)
        .expect("open store")
        .create_strand()
        .expect("create unlabeled room");

    let service = Service::open(
        service::Config {
            database_path: paths.database_path.display().to_string(),
            runtime_root: paths.runtime_root.display().to_string(),
            execution_root: paths.execution_root.display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
            constitution_path: None,
        },
        Arc::new(NoopProvider),
    )
    .expect("open service");
    assert_eq!(register_attempt_handover_budgets(&service).unwrap(), 1);
}

#[test]
fn rollback_detail_not_projected() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    let report = finalize_at(
        &paths,
        request(UpgradeTerminal::RolledBack {
            failure: UpgradeFailure {
                stage: UpgradeStage::Install,
                detail: "PACKAGE_SECRET_DETAIL".to_string(),
                recovery: RecoveryStatus::PreviousVersionRestored,
            },
        }),
    )
    .expect("finalize rollback");

    assert!(report.seeded);
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].code, "runtime.upgrade.failed");
    assert_eq!(report.errors[0].context["detail"], "PACKAGE_SECRET_DETAIL");
    assert!(!report.record.unwrap().contains("PACKAGE_SECRET_DETAIL"));

    let store = SantiStore::open(&paths.database_path).expect("open store");
    let incidents = store
        .error_incidents(&santi_core::Scope::new("runtime", "default"), 10)
        .expect("runtime errors");
    assert_eq!(incidents.len(), 1);
    assert_eq!(incidents[0].status, santi_core::Status::Active);
    assert!(!incidents[0].exposure.model);
}

#[test]
fn success_resolves_incident() {
    let temp = tempfile::tempdir().expect("temp dir");
    let paths = paths_under(temp.path());
    finalize_at(
        &paths,
        request(UpgradeTerminal::RolledBack {
            failure: UpgradeFailure {
                stage: UpgradeStage::TrialProbe,
                detail: "unhealthy".to_string(),
                recovery: RecoveryStatus::PreviousVersionRestored,
            },
        }),
    )
    .expect("finalize rollback");
    let recovered =
        finalize_at(&paths, request(UpgradeTerminal::Upgraded)).expect("finalize success");
    assert!(recovered.errors.is_empty());

    let store = SantiStore::open(&paths.database_path).expect("open store");
    let incidents = store
        .error_incidents(&santi_core::Scope::new("runtime", "default"), 10)
        .expect("runtime errors");
    assert_eq!(incidents.len(), 1);
    assert_eq!(incidents[0].status, santi_core::Status::Resolved);
    assert_eq!(incidents[0].revision, 2);
    assert_eq!(
        incidents[0].resolution.as_ref().unwrap().by.as_deref(),
        Some("upgrade.succeeded")
    );
}

mod handover;

fn request(terminal: UpgradeTerminal) -> UpgradeFinalizeRequest {
    request_for("upgrade_test", terminal)
}

fn request_for(attempt_id: &str, terminal: UpgradeTerminal) -> UpgradeFinalizeRequest {
    UpgradeFinalizeRequest {
        protocol_version: 1,
        attempt_id: attempt_id.to_string(),
        deb: "/tmp/santi.deb".to_string(),
        terminal,
        readiness: UpgradeReadiness::Ready,
        wake: true,
        soul_id: santi_core::DEFAULT_SOUL_ID.to_string(),
        configured_strand_id: None,
    }
}

fn paths_under(root: &Path) -> RuntimePaths {
    RuntimePaths {
        database_path: root.join("runtime").join("db"),
        runtime_root: root.join("runtime"),
        execution_root: root.join("execution"),
    }
}
