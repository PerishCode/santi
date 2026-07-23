use super::*;
use crate::ingest;

impl SantiStore {
    pub(crate) fn enqueue_inbox_with_context(
        &self,
        ingress: Ingress<'_>,
    ) -> Result<Intake, String> {
        self.enqueue_inbox_with_policy(ingress, true)
    }

    pub(crate) fn enqueue_inbox_while_suspended(
        &self,
        mut ingress: Ingress<'_>,
    ) -> Result<Intake, String> {
        ingress.admission = None;
        self.enqueue_inbox_with_policy(ingress, false)
    }

    fn enqueue_inbox_with_policy(
        &self,
        ingress: Ingress<'_>,
        enforce_active_holds: bool,
    ) -> Result<Intake, String> {
        let Ingress {
            strand,
            kind,
            content,
            source,
            admission,
            replay,
        } = ingress;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;

        if let Some(replay) = &replay
            && let Some((digest, strand, inbox)) =
                Database::new(&tx).replay(replay.owner, replay.request)?
        {
            tx.commit().map_err(|error| error.to_string())?;
            if digest != replay.digest {
                return Err("downstream request conflicts with an accepted payload".to_string());
            }
            return Ok(Intake {
                outcome: ingest::Outcome::Accepted {
                    receipt: ingest::Receipt {
                        strand,
                        inbox,
                        warning: None,
                    },
                },
                inserted: false,
            });
        }

        if enforce_active_holds
            && let Some(error) = crate::store::errors::drive::repeat_active_in_conn(
                &tx,
                strand,
                "ingest_active_guard",
            )?
        {
            tx.commit().map_err(|error| error.to_string())?;
            return Ok(Intake {
                outcome: ingest::Outcome::Rejected {
                    error: Box::new(error),
                },
                inserted: false,
            });
        }

        if enforce_active_holds
            && Database::new(&tx)
                .incident(&context_incident_key(strand))?
                .is_some()
        {
            let error = super::state::repeat_context_incident(
                &Database::new(&tx),
                strand,
                "ingest_active_guard",
            )?;
            tx.commit().map_err(|error| error.to_string())?;
            return Ok(Intake {
                outcome: ingest::Outcome::Rejected {
                    error: Box::new(error),
                },
                inserted: false,
            });
        }

        if let Some(admission) = admission {
            let database = Database::new(&tx);
            let mut input = assembly_input_in_conn(&tx, strand)?;
            input.extend(super::state::pending_items(&database, strand)?);
            if let Some(candidate) = crate::context::budget::inbound_provider_item(&kind, &content)
            {
                input.push(candidate);
            }
            let estimate = crate::context::budget::estimate_provider_parts(
                &input,
                admission.instructions.as_deref(),
                Some(admission.tools.as_slice()),
            );
            if estimate.total > admission.budget_bytes {
                let reason = over_budget_reason(estimate.total, admission.budget_bytes);
                let observed_at_seq = database.cursor(strand)?;
                let error = super::state::open_context_incident(
                    &database,
                    strand,
                    Pressure {
                        reason_code: "candidate_input_exceeds_budget",
                        reason_text: &reason,
                        operation: "ingest_admission",
                        provider: Some(&admission.provider),
                        model: Some(&admission.model),
                        budget_source: Some(&admission.budget_source),
                        budget_bytes: Some(admission.budget_bytes),
                        estimate: &estimate,
                        observed_turn_id: None,
                        observed_at_seq,
                        metadata: Some(json!({"estimator": estimate.estimator})),
                    },
                )?;
                tx.commit().map_err(|error| error.to_string())?;
                return Ok(Intake {
                    outcome: ingest::Outcome::Rejected {
                        error: Box::new(error),
                    },
                    inserted: false,
                });
            }
        }

        let pending: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM strand_inbox WHERE strand_id = ?1",
                params![strand],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if pending >= STRAND_INBOX_GATE {
            let message =
                format!("strand inbox is full ({pending} pending, gate {STRAND_INBOX_GATE})");
            let error = engine().transient(crate::Signal {
                descriptor: catalog::INBOX_CAPACITY_EXCEEDED,
                source: santi_error::Source::new("santi-core", "ingest_admission"),
                scope: Some(santi_error::Scope::new("strand", strand)),
                message,
                context: json!({"pending": pending, "gate": STRAND_INBOX_GATE}),
            });
            return Ok(Intake {
                outcome: ingest::Outcome::Rejected {
                    error: Box::new(error),
                },
                inserted: false,
            });
        }

        let inbox = tag("inbox");
        let now = now();
        let blob = serde_json::to_string(&content).map_err(|error| error.to_string())?;
        let origin = source.as_ref().map(|source| source.kind.as_str());
        let trace = source.as_ref().and_then(|source| source.source.as_deref());
        let metadata = source
            .as_ref()
            .and_then(|source| source.metadata.as_ref())
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        tx.execute(
            r#"
            INSERT INTO strand_inbox (
              id, strand_id, message_kind, content, source_type, source_ref, source_metadata, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                inbox,
                strand,
                kind.encode(),
                blob,
                origin,
                trace,
                metadata,
                now
            ],
        )
        .map_err(|error| error.to_string())?;
        Database::new(&tx).accept(&inbox, strand, &now)?;
        if let Some(replay) = replay {
            Database::new(&tx).stow(crate::store::db::Stowed {
                owner: replay.owner,
                request: replay.request,
                digest: replay.digest,
                strand,
                inbox: &inbox,
                created: &now,
            })?;
        }
        tx.commit().map_err(|error| error.to_string())?;
        Ok(Intake {
            outcome: ingest::Outcome::Accepted {
                receipt: ingest::Receipt {
                    strand: strand.to_string(),
                    inbox,
                    warning: None,
                },
            },
            inserted: true,
        })
    }
}
