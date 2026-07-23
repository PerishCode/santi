use super::*;

pub(super) fn finalize_handover(
    paths: &RuntimePaths,
    store: &santi_core::Store,
    request: UpgradeFinalizeRequest,
    mut errors: Vec<santi_core::Fault>,
) -> Result<UpgradeFinalizeReport, Box<santi_core::Fault>> {
    let record = compose_record(&request.attempt_id);
    let seed = paths.seed_attempt_handover(
        &request.soul,
        &request.attempt_id,
        request.configured_strand_id.as_deref(),
        &record,
    );
    let (seeded, seeded_strand_id, handover_failure) = match seed {
        Ok(seed) if seed.warnings.is_empty() => (true, Some(seed.strand), None),
        Ok(seed) => (true, Some(seed.strand), Some(seed.warnings.join("; "))),
        Err(error) => (false, None, Some(error)),
    };
    if let Some(detail) = handover_failure {
        let error = store
            .open_error_incident(santi_core::error::Draft {
                key: HANDOVER_INCIDENT_KEY.to_string(),
                descriptor: santi_core::catalog::UPGRADE_HANDOVER_FAILED,
                scope: santi_core::Scope::new("runtime", RUNTIME_SCOPE_ID),
                source: santi_core::Source::new("santi-api", "upgrade.handover"),
                message: "self-upgrade handover could not use its attempt-scoped ops strand"
                    .to_string(),
                context: json!({
                    "attempt_id": request.attempt_id,
                    "artifact": bounded(&request.deb),
                    "detail": bounded(&detail),
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
                    "artifact": bounded(&request.deb),
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

pub(super) fn attempt_ops_label(soul: &str, attempt_id: &str) -> String {
    format!("soul:{soul}:ops:upgrade:{attempt_id}")
}

pub(super) fn self_ops_label(soul: &str) -> String {
    format!("soul:{soul}:ops")
}

impl RuntimePaths {
    pub fn seed_come_look(
        &self,
        soul: &str,
        configured_strand: Option<&str>,
        text: &str,
    ) -> Result<SeedOutcome, String> {
        self.seed_handover_label(soul, &self_ops_label(soul), configured_strand, text)
    }

    pub fn seed_attempt_handover(
        &self,
        soul: &str,
        attempt_id: &str,
        configured_strand: Option<&str>,
        text: &str,
    ) -> Result<SeedOutcome, String> {
        self.seed_handover_label(
            soul,
            &attempt_ops_label(soul, attempt_id),
            configured_strand,
            text,
        )
    }

    fn seed_handover_label(
        &self,
        soul: &str,
        label: &str,
        configured_strand: Option<&str>,
        text: &str,
    ) -> Result<SeedOutcome, String> {
        let configured_strand = configured_strand
            .map(str::trim)
            .filter(|value| !value.is_empty());

        match self.inbox_seed_label(soul, label, text) {
            Ok(report) if report.accepted => Ok(SeedOutcome {
                strand: report.strand,
                warnings: Vec::new(),
            }),
            Ok(report) => self.seed_via_configured_strand(
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
            Err(error) => self.seed_via_configured_strand(
                configured_strand,
                text,
                format!("self-ops label {label} could not receive the come-look seed: {error}"),
            ),
        }
    }

    fn seed_via_configured_strand(
        &self,
        configured_strand: Option<&str>,
        text: &str,
        label_error: String,
    ) -> Result<SeedOutcome, String> {
        let Some(strand) = configured_strand else {
            return Err(label_error);
        };

        match self.inbox_seed(strand, text) {
            Ok(report) if report.accepted => Ok(SeedOutcome {
                strand: report.strand,
                warnings: vec![format!(
                    "{label_error}; fell back to configured SANTI_STRAND_ID {strand}"
                )],
            }),
            Ok(report) => Err(format!(
                "{label_error}; configured SANTI_STRAND_ID {strand} also rejected the come-look seed: {}",
                report
                    .error
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "seed rejected".to_string())
            )),
            Err(error) => Err(format!(
                "{label_error}; configured SANTI_STRAND_ID {strand} also could not receive the come-look seed: {error}"
            )),
        }
    }
}
