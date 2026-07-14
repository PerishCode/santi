mod bucket;
mod compact;
mod drive;
mod error;
mod flow;
mod fork;
mod im;
mod materials;
mod notice;
mod text;
mod thinking;
mod timing;
mod tools;

use santi_provider::ProviderClient;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::broadcast;

use crate::{
    CreateSoulRequest, CreateStrandResponse, CreateWebhookRequest, EffectResolutionOutcome,
    EffectStatus, ErrorIncident, ErrorScope, ErrorTransition, Execution, ReceiptStatus, SantiStore,
    SantiStreamEvent, SantiStreamPayload, Soul, Strand, StrandBudgetSnapshot, StrandDetail,
    StrandMaterial, StrandRuntimeSnapshot, WebhookSubscription, engine, prefixed_id, timestamp_now,
};

#[derive(Clone)]
pub struct Service {
    pub(crate) store: SantiStore,
    provider: Arc<dyn ProviderClient>,
    pub(crate) config: Config,
    material_cache: Arc<Mutex<HashMap<materials::Key, StrandMaterial>>>,
    stream_events: broadcast::Sender<SantiStreamEvent>,
    error_events: broadcast::Sender<ErrorTransition>,
    runtime_notices: notice::Bus,
    execution_budgets: Arc<Mutex<HashMap<String, Execution>>>,
    memory_pressure_lock: Arc<Mutex<()>>,
    shutting_down: Arc<AtomicBool>,
    drive_degraded: Arc<AtomicBool>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub database_path: String,
    pub runtime_root: String,
    pub execution_root: String,
    pub bind_addr: Option<String>,
}

impl Service {
    pub fn open(config: Config, provider: Arc<dyn ProviderClient>) -> Result<Self, String> {
        let store = SantiStore::open(&config.database_path)?;
        store.reconcile_orphaned_turns()?;
        let drive_degraded = store.active_drive_incident_count()? > 0;
        Ok(Self {
            store,
            provider,
            config,
            material_cache: Arc::new(Mutex::new(HashMap::new())),
            stream_events: broadcast::channel(1024).0,
            error_events: broadcast::channel(1024).0,
            runtime_notices: notice::Bus::new(),
            execution_budgets: Arc::new(Mutex::new(HashMap::new())),
            memory_pressure_lock: Arc::new(Mutex::new(())),
            shutting_down: Arc::new(AtomicBool::new(false)),
            drive_degraded: Arc::new(AtomicBool::new(drive_degraded)),
        })
    }

    pub fn set_strand_execution_budget(
        &self,
        strand_id: &str,
        budget: Execution,
    ) -> Result<(), String> {
        budget.validate()?;
        if self.store.strand(strand_id)?.is_none() {
            return Err("strand not found".to_string());
        }
        self.execution_budgets
            .lock()
            .unwrap()
            .insert(strand_id.to_string(), budget);
        Ok(())
    }

    pub(in crate::service) fn strand_execution_budget(&self, strand_id: &str) -> Option<Execution> {
        self.execution_budgets
            .lock()
            .unwrap()
            .get(strand_id)
            .cloned()
    }

    pub fn begin_shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
    }

    pub fn is_shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::SeqCst)
    }

    pub async fn drain_running_turns(&self, cap: Duration) {
        let start = Instant::now();
        loop {
            let remaining = match self.store.running_turn_count() {
                Ok(0) => return,
                Ok(remaining) => remaining,
                Err(error) => {
                    eprintln!("santi: shutdown drain scan failed: {error}");
                    return;
                }
            };
            if start.elapsed() >= cap {
                eprintln!(
                    "santi: shutdown drain cap reached with {remaining} turn(s) still running; leaving them to boot-recovery"
                );
                return;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    pub fn resume_pending(&self) -> Result<(), String> {
        self.dispatch_error_events();
        let pending = self.store.strands_with_pending_requests()?;
        for strand_id in pending {
            let outcome = self.poke(&strand_id, "strand_send", None, "cold_start_resume");
            if let drive::Outcome::Failed(error) = outcome
                && error.code == crate::catalog::ERROR_ENGINE_PERSISTENCE_FAILED.code
            {
                return Err(format!(
                    "cold-start recovery could not persist driver truth for strand {strand_id}: {}",
                    error.message
                ));
            }
        }
        Ok(())
    }

    pub fn is_drive_degraded(&self) -> bool {
        self.drive_degraded.load(Ordering::SeqCst)
    }

    pub fn active_drive_incident_count(&self) -> i64 {
        match self.store.active_drive_incident_count() {
            Ok(count) => count,
            Err(error) => {
                self.drive_degraded.store(true, Ordering::SeqCst);
                eprintln!("santi: drive health count failed: {error}");
                0
            }
        }
    }

    pub(in crate::service) fn mark_drive_degraded(&self) {
        self.drive_degraded.store(true, Ordering::SeqCst);
    }

    pub(in crate::service) fn refresh_drive_health(&self) {
        match self.store.active_drive_incident_count() {
            Ok(count) => self.drive_degraded.store(count > 0, Ordering::SeqCst),
            Err(error) => {
                self.drive_degraded.store(true, Ordering::SeqCst);
                eprintln!("santi: drive health refresh failed: {error}");
            }
        }
    }

    pub fn subscribe_stream(&self) -> broadcast::Receiver<SantiStreamEvent> {
        let receiver = self.stream_events.subscribe();
        self.dispatch_error_events();
        receiver
    }

    pub fn subscribe_error_transitions(&self) -> broadcast::Receiver<ErrorTransition> {
        let receiver = self.error_events.subscribe();
        self.dispatch_error_events();
        receiver
    }

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

    fn send_stream(&self, strand_id: &str, payload: SantiStreamPayload) -> Result<(), ()> {
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
