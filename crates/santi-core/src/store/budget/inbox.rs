use super::*;
use crate::Ruled;
use crate::ingest;

impl Store {
    pub(crate) fn ingest(&self, ingress: Ingress<'_>) -> Result<Intake, String> {
        self.intake(ingress, true)
    }

    pub(crate) fn harbor(&self, mut ingress: Ingress<'_>) -> Result<Intake, String> {
        ingress.admission = None;
        self.intake(ingress, false)
    }

    fn intake(&self, ingress: Ingress<'_>, enforce_active_holds: bool) -> Result<Intake, String> {
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
            && let Some((accepted, strand, inbox)) = replayed(&Database::new(&tx), replay)?
        {
            tx.commit().map_err(|error| error.to_string())?;
            if accepted != digest(replay) {
                return Err(conflict(replay).to_string());
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
            && let Some(error) =
                crate::store::errors::drive::stalled(&tx, strand, "ingest_active_guard")?
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
                .incident(
                    &crate::budget::Error::Context
                        .descriptor()
                        .key("strand", strand),
                )?
                .is_some()
        {
            let error = super::state::repress(&Database::new(&tx), strand, "ingest_active_guard")?;
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
            let mut input = assembled(&tx, strand)?;
            input.extend(super::state::queued(&database, strand)?);
            if let Some(candidate) = crate::context::budget::inbound(&kind, &content) {
                input.push(candidate);
            }
            let estimate = crate::context::budget::estimated(
                &input,
                admission.instructions.as_deref(),
                Some(admission.tools.as_slice()),
            );
            if estimate.total > admission.bytes {
                let reason = reason(estimate.total, admission.bytes);
                let observed = database.cursor(strand)?;
                let error = super::state::press(
                    &database,
                    strand,
                    Pressure {
                        code: "candidate_input_exceeds_budget",
                        text: &reason,
                        operation: "ingest_admission",
                        provider: Some(&admission.provider),
                        model: Some(&admission.model),
                        source: Some(&admission.source),
                        bytes: Some(admission.bytes),
                        estimate: &estimate,
                        observed: None,
                        at: observed,
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
        if pending >= GATE {
            let message = format!("strand inbox is full ({pending} pending, gate {GATE})");
            let error = engine().transient(crate::Signal {
                descriptor: crate::budget::Error::Inbox.descriptor(),
                source: santi_error::Source::new("santi-core", "ingest_admission"),
                scope: Some(santi_error::Scope::new("strand", strand)),
                message,
                context: json!({"pending": pending, "gate": GATE}),
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
            stow(
                &Database::new(&tx),
                replay,
                Accepted {
                    strand,
                    inbox: &inbox,
                    created: &now,
                },
            )?;
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

fn replayed(
    database: &Database<'_>,
    replay: &Replay<'_>,
) -> Result<Option<(String, String, String)>, String> {
    match replay {
        Replay::Downstream { owner, request, .. } => database.replay(owner, request),
        Replay::Webhook {
            subscription,
            delivery,
            ..
        } => database.delivery(subscription, delivery),
    }
}

fn digest<'a>(replay: &'a Replay<'_>) -> &'a str {
    match replay {
        Replay::Downstream { digest, .. } | Replay::Webhook { digest, .. } => digest,
    }
}

fn conflict(replay: &Replay<'_>) -> &'static str {
    match replay {
        Replay::Downstream { .. } => "downstream request conflicts with an accepted payload",
        Replay::Webhook { .. } => "webhook delivery conflicts with an accepted payload",
    }
}

struct Accepted<'a> {
    strand: &'a str,
    inbox: &'a str,
    created: &'a str,
}

fn stow(database: &Database<'_>, replay: Replay<'_>, accepted: Accepted<'_>) -> Result<(), String> {
    match replay {
        Replay::Downstream {
            owner,
            request,
            digest,
        } => database.stow(crate::store::db::Stowed {
            owner,
            request,
            digest,
            strand: accepted.strand,
            inbox: accepted.inbox,
            created: accepted.created,
        }),
        Replay::Webhook {
            subscription,
            delivery,
            digest,
        } => database.deliver(crate::store::db::Delivered {
            subscription,
            delivery,
            digest,
            strand: accepted.strand,
            inbox: accepted.inbox,
            created: accepted.created,
        }),
    }
}
