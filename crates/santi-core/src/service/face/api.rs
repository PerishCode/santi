use super::*;
use crate::{Incident, engine, now, soul::Soul, strand::Strand, tag};
use crate::{budget, effect, ingest, receipt, soul, strand, stream, webhook};

pub enum Admission {
    Accepted(ingest::Outcome),
    Denied,
    Forbidden,
}

impl Service {
    pub fn weave(&self) -> Result<strand::Created, String> {
        Ok(strand::Created {
            strand: self.store.weave()?,
        })
    }

    pub fn awaken(&self, request: soul::Draft) -> Result<Soul, String> {
        let soul = self.store.awaken()?;
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

    pub fn souls(&self) -> Result<Vec<Soul>, String> {
        self.store.souls()
    }

    pub fn soul(&self, soul: &str) -> Result<Option<Soul>, String> {
        self.store.soul(soul)
    }

    pub fn subscribe(&self, request: webhook::Draft) -> Result<webhook::Subscription, String> {
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
        if self.store.soul(soul)?.is_none() {
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
        self.store.subscribe(webhook::Draft {
            name: name.to_string(),
            adaptor: adaptor.to_string(),
            soul: soul.to_string(),
            strategy: Some(strategy.to_string()),
            credential: credential.to_string(),
        })
    }

    pub fn webhooks(&self) -> Result<Vec<webhook::Subscription>, String> {
        self.store.webhooks()
    }

    pub fn webhook(&self, name: &str) -> Result<Option<webhook::Subscription>, String> {
        self.store.webhook(name)
    }
    pub fn strands(&self) -> Result<Vec<Strand>, String> {
        self.store.strands()
    }

    pub fn strand(&self, strand: &str) -> Result<Option<strand::Detail>, String> {
        let Some(strand) = self.store.strand(strand)? else {
            return Ok(None);
        };
        Ok(Some(strand::Detail {
            messages: self.store.messages(&strand.id)?,
            strand,
        }))
    }

    pub fn snapshot(&self, strand: &str) -> Result<Option<stream::Snapshot>, String> {
        self.store.snapshot(strand)
    }

    pub fn audit(&self, strand: &str) -> Result<Option<budget::Snapshot>, String> {
        let Some(strand) = self.store.strand(strand)? else {
            return Ok(None);
        };
        Ok(Some(budget::Snapshot {
            strand: strand.id.clone(),
            estimate: self.estimate(&strand.id)?,
            budget: self.budget(),
            incident: self.store.pressure(&strand.id)?,
        }))
    }

    pub fn stranded(&self, strand: &str, limit: i64) -> Result<Option<Vec<Incident>>, String> {
        let Some(strand) = self.store.strand(strand)? else {
            return Ok(None);
        };
        self.store.stranded(&strand.id, limit).map(Some)
    }

    pub fn errors(&self, scope: &santi_error::Scope, limit: i64) -> Result<Vec<Incident>, String> {
        self.store.incidents(scope, limit)
    }

    pub fn receipt(&self, inbox: &str) -> Result<Option<receipt::Status>, String> {
        self.store.receipt(inbox)
    }

    pub fn effect(&self, effect: &str) -> Result<Option<effect::Status>, String> {
        self.store.effect(effect)
    }

    pub fn settle(
        &self,
        effect: &str,
        outcome: effect::Outcome,
        evidence: &str,
    ) -> Result<Option<effect::Status>, String> {
        self.store.settle(effect, outcome, evidence)
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

    pub fn since(
        &self,
        after_seq: i64,
        prefix: &str,
        limit: usize,
    ) -> Result<crate::event::Batch, String> {
        self.store.since(after_seq, prefix, limit)
    }

    pub(in crate::service) fn dispatched(&self) {
        let sink = error::Sink { service: self };
        if let Err(error) = engine().dispatch(&self.store, &sink, 256)
            && error != error::UNHEARD
        {
            eprintln!("santi: error outbox dispatch failed: {error}");
        }
    }
}
