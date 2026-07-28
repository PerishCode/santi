use super::*;
use crate::Ruled;
use crate::{Incident, now, soul::Soul, strand::Strand, tag};
use crate::{budget, effect, ingest, receipt, soul, strand, stream, webhook};

pub enum Admission {
    Accepted(ingest::Outcome),
    Denied,
    Forbidden,
}

impl Service {
    pub async fn weave(&self) -> Result<strand::Created, String> {
        let created = now();
        Ok(strand::Created {
            strand: self
                .store
                .create_strand(santi_estate::StrandDraft {
                    tag: &tag("ss"),
                    soul: crate::GENESIS,
                    label: None,
                    parent: None,
                    fork: None,
                    created: &created,
                })
                .await?,
        })
    }

    pub async fn awaken(&self, request: soul::Draft) -> Result<Soul, String> {
        let soul = self.store.create_soul(&tag("soul"), &now()).await?;
        if let Some(memory) = request
            .memory
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
        {
            let path = self.memoir(&soul.id);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            std::fs::write(&path, memory).map_err(|error| error.to_string())?;
        }
        Ok(soul)
    }

    pub async fn souls(&self) -> Result<Vec<Soul>, String> {
        self.store.souls().await
    }

    pub async fn soul(&self, soul: &str) -> Result<Option<Soul>, String> {
        self.store.soul(soul).await
    }

    pub async fn subscribe(
        &self,
        request: webhook::Draft,
    ) -> Result<webhook::Subscription, String> {
        let name = request.name.trim();
        let adaptor = request.adaptor.trim();
        let soul = request.soul.trim();
        let credential = request.credential.trim();
        if name.is_empty() {
            return Err("webhook name must not be empty".to_string());
        }
        if adaptor.is_empty() {
            return Err("webhook adaptor must not be empty".to_string());
        }
        if credential.is_empty() {
            return Err("webhook credential must not be empty".to_string());
        }
        if self.store.soul(soul).await?.is_none() {
            return Err("soul not found".to_string());
        }
        let strategy = request
            .strategy
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("per_thread");
        if !matches!(strategy, "per_thread" | "single") {
            return Err("strategy must be 'per_thread' or 'single'".to_string());
        }
        self.store
            .subscribe(santi_estate::WebhookDraft {
                name,
                adaptor,
                soul,
                strategy,
                credential,
                created: &now(),
            })
            .await
    }

    pub async fn webhooks(&self) -> Result<Vec<webhook::Subscription>, String> {
        self.store.webhooks().await
    }

    pub async fn webhook(&self, name: &str) -> Result<Option<webhook::Subscription>, String> {
        self.store.webhook(name).await
    }
    pub async fn strands(&self) -> Result<Vec<Strand>, String> {
        self.store.strands().await
    }

    pub async fn strand(&self, strand: &str) -> Result<Option<strand::Detail>, String> {
        let Some(strand) = self.store.strand(strand).await? else {
            return Ok(None);
        };
        Ok(Some(strand::Detail {
            messages: self.store.messages(&strand.id).await?,
            strand,
        }))
    }

    pub async fn snapshot(&self, strand: &str) -> Result<Option<stream::Snapshot>, String> {
        self.store.snapshot(strand).await
    }

    pub async fn audit(&self, strand: &str) -> Result<Option<budget::Snapshot>, String> {
        let Some(strand) = self.store.strand(strand).await? else {
            return Ok(None);
        };
        Ok(Some(budget::Snapshot {
            strand: strand.id.clone(),
            estimate: self.estimate(&strand.id).await?,
            budget: self.budget(),
            incident: self
                .store
                .incident(
                    &crate::budget::Error::Context
                        .descriptor()
                        .key("strand", &strand.id),
                )
                .await?,
        }))
    }

    pub async fn stranded(
        &self,
        strand: &str,
        limit: i64,
    ) -> Result<Option<Vec<Incident>>, String> {
        let Some(strand) = self.store.strand(strand).await? else {
            return Ok(None);
        };
        self.store
            .incidents(
                &santi_error::Scope::new("strand", &strand.id),
                usize::try_from(limit).unwrap_or(usize::MAX),
            )
            .await
            .map(Some)
    }

    pub async fn errors(
        &self,
        scope: &santi_error::Scope,
        limit: i64,
    ) -> Result<Vec<Incident>, String> {
        self.store
            .incidents(scope, usize::try_from(limit).unwrap_or(usize::MAX))
            .await
    }

    pub async fn receipt(&self, inbox: &str) -> Result<Option<receipt::Status>, String> {
        self.store.receipt(inbox).await
    }

    pub async fn effect(&self, effect: &str) -> Result<Option<effect::Status>, String> {
        self.store.effect(effect).await
    }

    pub async fn settle(
        &self,
        effect: &str,
        outcome: effect::Outcome,
        evidence: &str,
    ) -> Result<Option<effect::Status>, String> {
        self.store
            .settle_effect(effect, outcome, evidence, &now())
            .await
    }

    pub async fn trail(&self, effect: &str) -> Result<Vec<crate::trace::Record>, String> {
        self.store.traces("effect", effect).await
    }

    pub(crate) fn publish(&self, strand: &str, payload: stream::Payload) {
        let _ = self.streamed(strand, payload);
    }

    pub(in crate::service) fn streamed(
        &self,
        strand: &str,
        payload: stream::Payload,
    ) -> Result<(), ()> {
        self.streams
            .send(stream::Event {
                id: tag("stream"),
                strand: strand.to_string(),
                created: now(),
                payload,
            })
            .map(|_| ())
            .map_err(|_| ())
    }

    pub async fn since(
        &self,
        after_seq: i64,
        prefix: &str,
        limit: usize,
    ) -> Result<crate::event::Batch, String> {
        self.store.outbox("turns", after_seq, prefix, limit).await
    }

    pub(in crate::service) async fn dispatched(&self) {
        let sink = error::Sink { service: self };
        let transitions = match self.store.pending_errors(256).await {
            Ok(transitions) => transitions,
            Err(error) => {
                eprintln!("santi: error outbox read failed: {error}");
                return;
            }
        };
        for transition in transitions {
            if !self.dispatch_transition(&sink, &transition).await {
                break;
            }
        }
    }

    async fn dispatch_transition(
        &self,
        sink: &error::Sink<'_>,
        transition: &crate::Transition,
    ) -> bool {
        if let Err(error) = santi_error::Sink::publish(sink, transition) {
            if error != error::UNHEARD {
                eprintln!("santi: error outbox dispatch failed: {error}");
            }
            return false;
        }
        if let Err(error) = self.store.deliver_error(&transition.id, &now()).await {
            eprintln!("santi: error outbox acknowledgement failed: {error}");
            return false;
        }
        true
    }
}
