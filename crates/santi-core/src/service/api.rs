use crate::{
    CreateSoulRequest, CreateStrandResponse, CreateWebhookRequest, EffectResolutionOutcome,
    EffectStatus, ErrorIncident, ErrorScope, ReceiptStatus, SantiStreamEvent, SantiStreamPayload,
    Soul, Strand, StrandBudgetSnapshot, StrandDetail, StrandRuntimeSnapshot, WebhookSubscription,
    engine, prefixed_id, timestamp_now,
};

use super::*;

impl Service {
    pub fn create_strand(&self) -> Result<CreateStrandResponse, String> {
        Ok(CreateStrandResponse {
            strand: self.store.create_strand()?,
        })
    }

    pub fn create_soul(&self, request: CreateSoulRequest) -> Result<Soul, String> {
        let soul = self.store.create_soul()?;
        if let Some(memory) = request
            .memory
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
        {
            let path = self.soul_memory_file(&soul.id);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            std::fs::write(&path, memory).map_err(|error| error.to_string())?;
        }
        Ok(soul)
    }

    pub fn list_souls(&self) -> Result<Vec<Soul>, String> {
        self.store.list_souls()
    }

    pub fn soul(&self, soul_id: &str) -> Result<Option<Soul>, String> {
        self.store.soul(soul_id)
    }

    pub fn create_webhook(
        &self,
        request: CreateWebhookRequest,
    ) -> Result<WebhookSubscription, String> {
        let name = request.name.trim();
        let adaptor = request.adaptor.trim();
        let soul_id = request.soul_id.trim();
        let secret_env = request.secret_env.trim();
        if name.is_empty() {
            return Err("webhook name must not be empty".to_string());
        }
        if adaptor.is_empty() {
            return Err("webhook adaptor must not be empty".to_string());
        }
        if secret_env.is_empty() {
            return Err("webhook secret_env must not be empty".to_string());
        }
        if self.store.soul(soul_id)?.is_none() {
            return Err("soul not found".to_string());
        }
        let strand_strategy = request
            .strand_strategy
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("per_thread");
        if !matches!(strand_strategy, "per_thread" | "single") {
            return Err("strand_strategy must be 'per_thread' or 'single'".to_string());
        }
        self.store.create_webhook(CreateWebhookRequest {
            name: name.to_string(),
            adaptor: adaptor.to_string(),
            soul_id: soul_id.to_string(),
            strand_strategy: Some(strand_strategy.to_string()),
            secret_env: secret_env.to_string(),
        })
    }

    pub fn list_webhooks(&self) -> Result<Vec<WebhookSubscription>, String> {
        self.store.list_webhooks()
    }

    pub fn webhook(&self, name: &str) -> Result<Option<WebhookSubscription>, String> {
        self.store.webhook(name)
    }
    pub fn list_strands(&self) -> Result<Vec<Strand>, String> {
        self.store.list_strands()
    }

    pub fn strand(&self, strand_id: &str) -> Result<Option<StrandDetail>, String> {
        let Some(strand) = self.store.strand(strand_id)? else {
            return Ok(None);
        };
        Ok(Some(StrandDetail {
            messages: self.store.strand_messages(strand_id)?,
            strand,
        }))
    }

    pub fn runtime_snapshot(
        &self,
        strand_id: &str,
    ) -> Result<Option<StrandRuntimeSnapshot>, String> {
        self.store.runtime_snapshot(strand_id)
    }

    pub fn strand_budget(&self, strand_id: &str) -> Result<Option<StrandBudgetSnapshot>, String> {
        let Some(strand) = self.store.strand(strand_id)? else {
            return Ok(None);
        };
        Ok(Some(StrandBudgetSnapshot {
            strand_id: strand.id.clone(),
            estimate: self.current_context_estimate(&strand.id)?,
            budget: self.context_budget(),
            active_incident: self.store.active_context_incident(&strand.id)?,
        }))
    }

    pub fn strand_errors(
        &self,
        strand_id: &str,
        limit: i64,
    ) -> Result<Option<Vec<ErrorIncident>>, String> {
        let Some(strand) = self.store.strand(strand_id)? else {
            return Ok(None);
        };
        self.store
            .error_incidents_for_strand(&strand.id, limit)
            .map(Some)
    }

    pub fn errors(&self, scope: &ErrorScope, limit: i64) -> Result<Vec<ErrorIncident>, String> {
        self.store.error_incidents(scope, limit)
    }

    pub fn receipt_status(&self, inbox_id: &str) -> Result<Option<ReceiptStatus>, String> {
        self.store.receipt_status(inbox_id)
    }

    pub fn im_deliveries_for_receipt(
        &self,
        inbox_id: &str,
    ) -> Result<Vec<crate::ImDelivery>, String> {
        self.store.im_deliveries_for_receipt(inbox_id)
    }

    pub fn effect_status(&self, effect_id: &str) -> Result<Option<EffectStatus>, String> {
        self.store.effect_status(effect_id)
    }

    pub fn resolve_effect(
        &self,
        effect_id: &str,
        outcome: EffectResolutionOutcome,
        evidence: &str,
    ) -> Result<Option<EffectStatus>, String> {
        self.store.resolve_effect(effect_id, outcome, evidence)
    }

    pub(crate) fn publish_stream(&self, strand_id: &str, payload: SantiStreamPayload) {
        let _ = self.send_stream(strand_id, payload);
    }

    pub(super) fn send_stream(
        &self,
        strand_id: &str,
        payload: SantiStreamPayload,
    ) -> Result<(), ()> {
        self.stream_events
            .send(SantiStreamEvent {
                event_id: prefixed_id("stream"),
                strand_id: strand_id.to_string(),
                created_at: timestamp_now(),
                payload,
            })
            .map(|_| ())
            .map_err(|_| ())
    }

    pub(in crate::service) fn dispatch_error_events(&self) {
        let sink = error::Sink { service: self };
        if let Err(error) = engine().dispatch_outbox(&self.store, &sink, 256)
            && error != error::NO_ERROR_EVENT_SUBSCRIBERS
        {
            eprintln!("santi: error outbox dispatch failed: {error}");
        }
    }
}
