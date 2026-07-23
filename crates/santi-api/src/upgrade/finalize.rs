use serde_json::json;

use super::{
    SeedOutcome, UpgradeFinalizeReport, UpgradeFinalizeRequest, UpgradeTerminal, compose_record,
};
use crate::config::RuntimePaths;

pub(super) const FINALIZE_PROTOCOL_VERSION: u32 = 1;
const RUNTIME_SCOPE_ID: &str = "default";
const UPGRADE_INCIDENT_KEY: &str = "runtime.upgrade.failed:runtime:default";
const HANDOVER_INCIDENT_KEY: &str = "runtime.upgrade.handover_failed:runtime:default";

pub(super) fn persistence_error(
    attempt_id: &str,
    deb: &str,
    operation: &str,
    detail: impl Into<String>,
) -> santi_core::Fault {
    santi_core::engine().transient(santi_core::Signal {
        descriptor: santi_core::catalog::ERROR_ENGINE_PERSISTENCE_FAILED,
        source: santi_core::Source::new("santi-api", operation),
        scope: Some(santi_core::Scope::new("runtime", RUNTIME_SCOPE_ID)),
        message: "error engine could not persist self-upgrade terminal truth".to_string(),
        context: json!({
            "attempt_id": attempt_id,
            "artifact": bounded(deb),
            "detail": bounded(&detail.into()),
        }),
    })
}

fn bounded(value: &str) -> String {
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
) -> Result<UpgradeFinalizeReport, Box<santi_core::Fault>> {
    let paths = crate::runtime::held().paths.clone();
    finalize_at(&paths, request)
}

pub fn finalize_at(
    paths: &RuntimePaths,
    request: UpgradeFinalizeRequest,
) -> Result<UpgradeFinalizeReport, Box<santi_core::Fault>> {
    if request.protocol_version != FINALIZE_PROTOCOL_VERSION {
        return Err(Box::new(santi_core::engine().transient(
            santi_core::Signal {
                descriptor: santi_core::catalog::INVALID_ARGUMENT,
                source: santi_core::Source::new("santi-api", "upgrade.finalize"),
                scope: Some(santi_core::Scope::new("runtime", RUNTIME_SCOPE_ID)),
                message: "unsupported upgrade finalization protocol".to_string(),
                context: json!({
                    "expected": FINALIZE_PROTOCOL_VERSION,
                    "actual": request.protocol_version,
                }),
            },
        )));
    }

    let store = santi_core::Store::open(&paths.database).map_err(|error| {
        Box::new(persistence_error(
            &request.attempt_id,
            &request.deb,
            "upgrade.finalize.open_store",
            error,
        ))
    })?;
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
            errors.push(open_execution_failure(&store, &request, failure, terminal)?);
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
    store: &santi_core::Store,
    request: &UpgradeFinalizeRequest,
    readiness: super::UpgradeReadiness,
) -> Result<(), Box<santi_core::Fault>> {
    store
        .resolve_error_incident(
            UPGRADE_INCIDENT_KEY,
            "upgrade.succeeded",
            json!({
                "attempt_id": request.attempt_id,
                "artifact": bounded(&request.deb),
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
    store: &santi_core::Store,
    request: &UpgradeFinalizeRequest,
    failure: &super::UpgradeFailure,
    terminal: &str,
) -> Result<santi_core::Fault, Box<santi_core::Fault>> {
    store
        .open_error_incident(santi_core::error::Draft {
            key: UPGRADE_INCIDENT_KEY.to_string(),
            descriptor: santi_core::catalog::UPGRADE_FAILED,
            scope: santi_core::Scope::new("runtime", RUNTIME_SCOPE_ID),
            source: santi_core::Source::new("santi-api", failure.stage.operation()),
            message: format!("self-upgrade failed during {}", failure.stage.operation()),
            context: json!({
                "attempt_id": request.attempt_id,
                "artifact": bounded(&request.deb),
                "terminal": terminal,
                "stage": failure.stage,
                "detail": bounded(&failure.detail),
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

mod handover;
use handover::*;
