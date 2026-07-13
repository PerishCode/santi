use serde_json::json;

use super::{
    SeedOutcome, UpgradeFinalizeReport, UpgradeFinalizeRequest, UpgradeTerminal, compose_record,
};
use crate::config::{self, RuntimePaths};

pub(super) const FINALIZE_PROTOCOL_VERSION: u32 = 1;
const RUNTIME_SCOPE_ID: &str = "default";
const UPGRADE_INCIDENT_KEY: &str = "runtime.upgrade.failed:runtime:default";
const HANDOVER_INCIDENT_KEY: &str = "runtime.upgrade.handover_failed:runtime:default";

pub(super) fn persistence_error(
    attempt_id: &str,
    deb: &str,
    operation: &str,
    detail: impl Into<String>,
) -> santi_core::SantiError {
    santi_core::engine().transient(
        santi_core::catalog::ERROR_ENGINE_PERSISTENCE_FAILED,
        santi_core::ErrorSource::new("santi-api", operation),
        Some(santi_core::ErrorScope::new("runtime", RUNTIME_SCOPE_ID)),
        "error engine could not persist self-upgrade terminal truth",
        json!({
            "attempt_id": attempt_id,
            "artifact": bounded_detail(deb),
            "detail": bounded_detail(&detail.into()),
        }),
    )
}

fn bounded_detail(value: &str) -> String {
    const LIMIT: usize = 4096;
    let mut chars = value.chars();
    let bounded: String = chars.by_ref().take(LIMIT).collect();
    if chars.next().is_some() {
        format!("{bounded} [truncated]")
    } else {
        bounded
    }
}

pub fn finalize(
    request: UpgradeFinalizeRequest,
) -> Result<UpgradeFinalizeReport, Box<santi_core::SantiError>> {
    let paths = config::resolve_runtime_paths();
    finalize_at(&paths, request)
}

pub fn finalize_at(
    paths: &RuntimePaths,
    request: UpgradeFinalizeRequest,
) -> Result<UpgradeFinalizeReport, Box<santi_core::SantiError>> {
    if request.protocol_version != FINALIZE_PROTOCOL_VERSION {
        return Err(Box::new(santi_core::engine().transient(
            santi_core::catalog::INVALID_ARGUMENT,
            santi_core::ErrorSource::new("santi-api", "upgrade.finalize"),
            Some(santi_core::ErrorScope::new("runtime", RUNTIME_SCOPE_ID)),
            "unsupported upgrade finalization protocol",
            json!({
                "expected": FINALIZE_PROTOCOL_VERSION,
                "actual": request.protocol_version,
            }),
        )));
    }

    let store = santi_core::SantiStore::open(&paths.database_path).map_err(|error| {
        Box::new(persistence_error(
            &request.attempt_id,
            &request.deb,
            "upgrade.finalize.open_store",
            error,
        ))
    })?;
    let scope = santi_core::ErrorScope::new("runtime", RUNTIME_SCOPE_ID);
    let mut errors = Vec::new();

    match &request.terminal {
        UpgradeTerminal::Upgraded => {
            resolve_upgrade(&store, &request, request.readiness)?;
        }
        UpgradeTerminal::RolledBack { failure } | UpgradeTerminal::Failed { failure } => {
            let terminal = if matches!(request.terminal, UpgradeTerminal::RolledBack { .. }) {
                "rolled_back"
            } else {
                "failed"
            };
            errors.push(open_execution_failure(
                &store, &scope, &request, failure, terminal,
            )?);
        }
    }

    if !request.wake {
        return Ok(UpgradeFinalizeReport {
            errors,
            record: None,
            seeded: false,
            seeded_strand_id: None,
        });
    }

    finalize_handover(paths, &store, request, errors)
}

fn resolve_upgrade(
    store: &santi_core::SantiStore,
    request: &UpgradeFinalizeRequest,
    readiness: super::UpgradeReadiness,
) -> Result<(), Box<santi_core::SantiError>> {
    store
        .resolve_error_incident(
            UPGRADE_INCIDENT_KEY,
            "upgrade.succeeded",
            json!({
                "attempt_id": request.attempt_id,
                "artifact": bounded_detail(&request.deb),
                "terminal": if matches!(readiness, super::UpgradeReadiness::Degraded) {
                    "upgraded_degraded"
                } else {
                    "upgraded"
                },
                "readiness": readiness,
            }),
        )
        .map(|_| ())
        .map_err(|error| {
            Box::new(persistence_error(
                &request.attempt_id,
                &request.deb,
                "upgrade.finalize.resolve_execution",
                error,
            ))
        })
}

fn open_execution_failure(
    store: &santi_core::SantiStore,
    scope: &santi_core::ErrorScope,
    request: &UpgradeFinalizeRequest,
    failure: &super::UpgradeFailure,
    terminal: &str,
) -> Result<santi_core::SantiError, Box<santi_core::SantiError>> {
    store
        .open_error_incident(santi_core::IncidentDraft {
            incident_key: UPGRADE_INCIDENT_KEY.to_string(),
            descriptor: santi_core::catalog::UPGRADE_FAILED,
            scope: scope.clone(),
            source: santi_core::ErrorSource::new("santi-api", failure.stage.operation()),
            message: format!("self-upgrade failed during {}", failure.stage.operation()),
            context: json!({
                "attempt_id": request.attempt_id,
                "artifact": bounded_detail(&request.deb),
                "terminal": terminal,
                "stage": failure.stage,
                "detail": bounded_detail(&failure.detail),
                "recovery": failure.recovery,
            }),
        })
        .map_err(|error| {
            Box::new(persistence_error(
                &request.attempt_id,
                &request.deb,
                "upgrade.finalize.open_execution",
                error,
            ))
        })
}

fn finalize_handover(
    paths: &RuntimePaths,
    store: &santi_core::SantiStore,
    request: UpgradeFinalizeRequest,
    mut errors: Vec<santi_core::SantiError>,
) -> Result<UpgradeFinalizeReport, Box<santi_core::SantiError>> {
    let record = compose_record(&request.attempt_id);
    let seed = seed_attempt_handover_at(
        paths,
        &request.soul_id,
        &request.attempt_id,
        request.configured_strand_id.as_deref(),
        &record,
    );
    let (seeded, seeded_strand_id, handover_failure) = match seed {
        Ok(seed) if seed.warnings.is_empty() => (true, Some(seed.strand_id), None),
        Ok(seed) => (true, Some(seed.strand_id), Some(seed.warnings.join("; "))),
        Err(error) => (false, None, Some(error)),
    };
    if let Some(detail) = handover_failure {
        let error = store
            .open_error_incident(santi_core::IncidentDraft {
                incident_key: HANDOVER_INCIDENT_KEY.to_string(),
                descriptor: santi_core::catalog::UPGRADE_HANDOVER_FAILED,
                scope: santi_core::ErrorScope::new("runtime", RUNTIME_SCOPE_ID),
                source: santi_core::ErrorSource::new("santi-api", "upgrade.handover"),
                message: "self-upgrade handover could not use its attempt-scoped ops strand"
                    .to_string(),
                context: json!({
                    "attempt_id": request.attempt_id,
                    "artifact": bounded_detail(&request.deb),
                    "detail": bounded_detail(&detail),
                    "seeded": seeded,
                    "seeded_strand_id": seeded_strand_id,
                }),
            })
            .map_err(|error| {
                Box::new(persistence_error(
                    &request.attempt_id,
                    &request.deb,
                    "upgrade.finalize.open_handover",
                    error,
                ))
            })?;
        errors.push(error);
    } else {
        store
            .resolve_error_incident(
                HANDOVER_INCIDENT_KEY,
                "upgrade.handover_succeeded",
                json!({
                    "attempt_id": request.attempt_id,
                    "artifact": bounded_detail(&request.deb),
                    "seeded_strand_id": seeded_strand_id,
                }),
            )
            .map_err(|error| {
                Box::new(persistence_error(
                    &request.attempt_id,
                    &request.deb,
                    "upgrade.finalize.resolve_handover",
                    error,
                ))
            })?;
    }

    Ok(UpgradeFinalizeReport {
        errors,
        record: Some(record),
        seeded,
        seeded_strand_id,
    })
}

fn attempt_ops_label(soul_id: &str, attempt_id: &str) -> String {
    format!("soul:{soul_id}:ops:upgrade:{attempt_id}")
}

fn self_ops_label(soul_id: &str) -> String {
    format!("soul:{soul_id}:ops")
}

/// Preserve the original stable-label seed helper for callers that explicitly
/// want one long-lived ops room. Upgrade finalization uses the attempt-scoped
/// variant below so audit scratch output cannot accumulate across releases.
pub fn seed_come_look_at(
    paths: &RuntimePaths,
    soul_id: &str,
    configured_strand: Option<&str>,
    text: &str,
) -> Result<SeedOutcome, String> {
    seed_handover_label_at(
        paths,
        soul_id,
        &self_ops_label(soul_id),
        configured_strand,
        text,
    )
}

pub fn seed_attempt_handover_at(
    paths: &RuntimePaths,
    soul_id: &str,
    attempt_id: &str,
    configured_strand: Option<&str>,
    text: &str,
) -> Result<SeedOutcome, String> {
    seed_handover_label_at(
        paths,
        soul_id,
        &attempt_ops_label(soul_id, attempt_id),
        configured_strand,
        text,
    )
}

fn seed_handover_label_at(
    paths: &RuntimePaths,
    soul_id: &str,
    label: &str,
    configured_strand: Option<&str>,
    text: &str,
) -> Result<SeedOutcome, String> {
    let configured_strand = configured_strand
        .map(str::trim)
        .filter(|value| !value.is_empty());

    match crate::ops::inbox_seed_label_at(paths, soul_id, label, text) {
        Ok(report) if report.accepted => Ok(SeedOutcome {
            strand_id: report.strand_id,
            warnings: Vec::new(),
        }),
        Ok(report) => seed_via_configured_strand(
            paths,
            configured_strand,
            text,
            format!(
                "self-ops label {label} rejected the come-look seed: {}",
                report
                    .error
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "seed rejected".to_string())
            ),
        ),
        Err(error) => seed_via_configured_strand(
            paths,
            configured_strand,
            text,
            format!("self-ops label {label} could not receive the come-look seed: {error}"),
        ),
    }
}

fn seed_via_configured_strand(
    paths: &RuntimePaths,
    configured_strand: Option<&str>,
    text: &str,
    label_error: String,
) -> Result<SeedOutcome, String> {
    let Some(strand_id) = configured_strand else {
        return Err(label_error);
    };

    match crate::ops::inbox_seed_at(paths, strand_id, text) {
        Ok(report) if report.accepted => Ok(SeedOutcome {
            strand_id: report.strand_id,
            warnings: vec![format!(
                "{label_error}; fell back to configured SANTI_STRAND_ID {strand_id}"
            )],
        }),
        Ok(report) => Err(format!(
            "{label_error}; configured SANTI_STRAND_ID {strand_id} also rejected the come-look seed: {}",
            report
                .error
                .map(|error| error.to_string())
                .unwrap_or_else(|| "seed rejected".to_string())
        )),
        Err(error) => Err(format!(
            "{label_error}; configured SANTI_STRAND_ID {strand_id} also could not receive the come-look seed: {error}"
        )),
    }
}
