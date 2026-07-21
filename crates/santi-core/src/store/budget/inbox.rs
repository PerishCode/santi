use super::*;

impl SantiStore {
    pub(crate) fn enqueue_inbox_with_context(
        &self,
        ingress: Ingress<'_>,
    ) -> Result<IngestOutcome, String> {
        self.enqueue_inbox_with_policy(ingress, true)
    }

    pub(crate) fn enqueue_inbox_while_suspended(
        &self,
        strand_id: &str,
        message_kind: MessageKind,
        content: MessageContent,
        source: Option<InboxSource>,
    ) -> Result<IngestOutcome, String> {
        self.enqueue_inbox_with_policy(
            Ingress {
                strand: strand_id,
                kind: message_kind,
                content,
                source,
                admission: None,
                window: None,
            },
            false,
        )
    }

    fn enqueue_inbox_with_policy(
        &self,
        ingress: Ingress<'_>,
        enforce_active_holds: bool,
    ) -> Result<IngestOutcome, String> {
        let Ingress {
            strand,
            kind,
            content,
            source,
            admission,
            window,
        } = ingress;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;

        if enforce_active_holds
            && let Some(error) = crate::store::errors::drive::repeat_active_in_conn(
                &tx,
                strand,
                "ingest_active_guard",
            )?
        {
            tx.commit().map_err(|error| error.to_string())?;
            return Ok(IngestOutcome::Rejected {
                error: Box::new(error),
            });
        }

        if enforce_active_holds
            && Database::new(&tx)
                .active_incident(&context_incident_key(strand))?
                .is_some()
        {
            let error = super::state::repeat_context_incident(
                &Database::new(&tx),
                strand,
                "ingest_active_guard",
            )?;
            tx.commit().map_err(|error| error.to_string())?;
            return Ok(IngestOutcome::Rejected {
                error: Box::new(error),
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
            if estimate.total_bytes > admission.budget_bytes {
                let reason = over_budget_reason(estimate.total_bytes, admission.budget_bytes);
                let observed_at_seq = database.current_strand_seq(strand)?;
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
                return Ok(IngestOutcome::Rejected {
                    error: Box::new(error),
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
                source: ErrorSource::new("santi-core", "ingest_admission"),
                scope: Some(ErrorScope::new("strand", strand)),
                message,
                context: json!({"pending": pending, "gate": STRAND_INBOX_GATE}),
            });
            return Ok(IngestOutcome::Rejected {
                error: Box::new(error),
            });
        }

        let inbox_id = prefixed_id("inbox");
        let now = timestamp_now();
        let content_json = serde_json::to_string(&content).map_err(|error| error.to_string())?;
        let source_type = source.as_ref().map(|source| source.source_type.as_str());
        let source_ref = source
            .as_ref()
            .and_then(|source| source.source_ref.as_deref());
        let source_metadata = source
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
                inbox_id,
                strand,
                kind.encode(),
                content_json,
                source_type,
                source_ref,
                source_metadata,
                now
            ],
        )
        .map_err(|error| error.to_string())?;
        if let Some(reservation) = window {
            tx.execute(
                r#"
                INSERT INTO window_messages (
                  participant_id, client_message_id, inbox_id, message_id, content_hash, cursor, received_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6)
                "#,
                params![
                    reservation.participant,
                    reservation.client,
                    inbox_id,
                    reservation.message,
                    reservation.hash,
                    reservation.received
                ],
            )
            .map_err(|error| error.to_string())?;
        }
        Database::new(&tx).insert_accepted(&inbox_id, strand, &now)?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(IngestOutcome::Accepted {
            receipt: IngestReceipt {
                strand_id: strand.to_string(),
                inbox_id,
                warning: None,
            },
        })
    }
}
