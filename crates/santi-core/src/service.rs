mod compact;
mod flow;
mod fork;
mod im;
mod materials;
mod runtime_notice;
mod text_delta;
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
    CreateSoulRequest, CreateStrandResponse, CreateWebhookRequest, ErrorEventSink, ErrorIncident,
    ErrorScope, ErrorTransition, MaterialKind, SantiError, SantiStore, SantiStreamEvent,
    SantiStreamPayload, Soul, Strand, StrandBudgetSnapshot, StrandDetail, StrandMaterial,
    StrandMessage, StrandRuntimeSnapshot, Turn, WebhookSubscription, engine, prefixed_id,
    timestamp_now,
};
use runtime_notice::RuntimeNoticeBus;

#[derive(Clone)]
pub struct SantiService {
    pub(crate) store: SantiStore,
    provider: Arc<dyn ProviderClient>,
    pub(crate) config: SantiServiceConfig,
    material_cache: Arc<Mutex<HashMap<MaterialCacheKey, StrandMaterial>>>,
    stream_events: broadcast::Sender<SantiStreamEvent>,
    error_events: broadcast::Sender<ErrorTransition>,
    runtime_notices: RuntimeNoticeBus,
    /// Graceful-shutdown latch (PHASE-07): once set, `poke` refuses to START new
    /// turns, so inbox CONSUMPTION pauses while ingest keeps durably enqueuing
    /// (the inbox is an MQ — we stop consuming, never producing). The in-flight
    /// turn is left to finish; `drain_running_turns` waits it out.
    shutting_down: Arc<AtomicBool>,
    drive_degraded: Arc<AtomicBool>,
}

type MaterialCacheKey = (String, MaterialKind);
/// The complete result of one driver poke, including normal no-start states and
/// canonical holds/failures that transports must surface without losing the
/// durable enqueue truth.
pub(crate) enum DriveOutcome {
    Started(Turn, Vec<StrandMessage>),
    Running(Turn),
    Idle,
    Held(SantiError),
    Paused,
    Failed(SantiError),
}

const NO_ERROR_EVENT_SUBSCRIBERS: &str = "error event bus has no subscribers";

struct StreamErrorSink<'a> {
    service: &'a SantiService,
}

impl ErrorEventSink for StreamErrorSink<'_> {
    fn publish_error_transition(&self, transition: &ErrorTransition) -> Result<(), String> {
        let strand_delivered = transition.incident.scope.kind == "strand"
            && self
                .service
                .send_stream(
                    &transition.incident.scope.id,
                    SantiStreamPayload::ErrorTransition {
                        transition: Box::new(transition.clone()),
                    },
                )
                .is_ok();
        let global_delivered = self.service.error_events.send(transition.clone()).is_ok();
        if strand_delivered || global_delivered {
            Ok(())
        } else {
            Err(NO_ERROR_EVENT_SUBSCRIBERS.to_string())
        }
    }
}

#[derive(Debug, Clone)]
pub struct SantiServiceConfig {
    pub database_path: String,
    pub runtime_root: String,
    pub execution_root: String,
    pub bind_addr: Option<String>,
}

impl SantiService {
    pub fn open(
        config: SantiServiceConfig,
        provider: Arc<dyn ProviderClient>,
    ) -> Result<Self, String> {
        let store = SantiStore::open(&config.database_path)?;
        // Boot recovery (honest occurrence): any turn still `running` is orphaned
        // by the restart — reconcile it to an interrupted terminal so the soul
        // sees the truth and its strand is idle again. Re-driving stranded
        // requests is liveness; call `resume_pending` once inside the runtime.
        store.reconcile_orphaned_turns()?;
        let drive_degraded = store.active_drive_incident_count()? > 0;
        Ok(Self {
            store,
            provider,
            config,
            material_cache: Arc::new(Mutex::new(HashMap::new())),
            stream_events: broadcast::channel(1024).0,
            error_events: broadcast::channel(1024).0,
            runtime_notices: RuntimeNoticeBus::new(),
            shutting_down: Arc::new(AtomicBool::new(false)),
            drive_degraded: Arc::new(AtomicBool::new(drive_degraded)),
        })
    }

    /// Begin a graceful shutdown: stop consuming the inbox (no new turns start).
    /// Idempotent. Ingest still durably enqueues; the in-flight turn finishes.
    pub fn begin_shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
    }

    pub fn is_shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::SeqCst)
    }

    /// Wait until no turn is `running` (the in-flight turn finished), or until
    /// `cap` elapses. Called after the HTTP server has stopped accepting, so no
    /// new turns can appear once shutdown has begun. On cap-timeout it returns
    /// anyway: the still-running turn will be reconciled to `interrupted` on the
    /// next boot (honest occurrence), and the external upgrade flow's own bound
    /// (SIGKILL) is the hard stop.
    pub async fn drain_running_turns(&self, cap: Duration) {
        let start = Instant::now();
        loop {
            match self.store.running_turn_count() {
                Ok(0) => return,
                Ok(remaining) => {
                    if start.elapsed() >= cap {
                        eprintln!(
                            "santi: shutdown drain cap reached with {remaining} turn(s) still running; leaving them to boot-recovery"
                        );
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                Err(error) => {
                    eprintln!("santi: shutdown drain scan failed: {error}");
                    return;
                }
            }
        }
    }

    /// Re-drive strands left "behind" by a crash (their inbox durably holds
    /// content nobody ever drained). Liveness only — no retry of
    /// attempted/failed turns. Call once at server startup (inside the tokio
    /// runtime).
    pub fn resume_pending(&self) -> Result<(), String> {
        self.dispatch_error_events();
        let pending = self.store.strands_with_pending_requests()?;
        for strand_id in pending {
            let outcome = self.poke(&strand_id, "strand_send", None, "cold_start_resume");
            if let DriveOutcome::Failed(error) = outcome
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

    /// Create a soul and seed its initial `[santi-soul]` memory. A soul is
    /// id-only; its identity IS its memory, so creation optionally carries the
    /// starting memory to write into the soul's memory file (absent → a blank
    /// soul that will author its own).
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
        self.store
            .create_webhook(name, adaptor, soul_id, strand_strategy, secret_env)
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
        let sink = StreamErrorSink { service: self };
        if let Err(error) = engine().dispatch_outbox(&self.store, &sink, 256)
            && error != NO_ERROR_EVENT_SUBSCRIBERS
        {
            eprintln!("santi: error outbox dispatch failed: {error}");
        }
    }
}
