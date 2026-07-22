mod engine;
use engine::{address, drive, error, materials, notice, thinking, timing};
mod face;
pub use face::Admission;
mod flow;
mod text;
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

use crate::{ErrorTransition, Execution, SantiStore, SantiStreamEvent, StrandMaterial};

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
}
