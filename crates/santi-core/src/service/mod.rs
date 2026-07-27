mod engine;
use engine::{address, drive, error, materials, notice, thinking, timing};
mod face;
pub use face::Admission;
mod flow;
mod jobs;
pub use jobs::{
    Draft as JobDraft, Launch as JobLaunch, Observation as JobObservation, Read as JobRead,
    Supervisor as JobSupervisor, Terminal as JobTerminal,
};
mod text;
mod tools;

use santi_provider::Provider;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::broadcast;

use crate::{Store, Transition};
use crate::{budget, material, stream};

#[derive(Clone)]
pub struct Service {
    pub(crate) store: Store,
    pub(crate) context: plumb::context::Context,
    provider: Arc<dyn Provider>,
    pub(crate) config: Config,
    materials: Arc<Mutex<HashMap<materials::Key, material::Material>>>,
    streams: broadcast::Sender<stream::Event>,
    errors: broadcast::Sender<Transition>,
    notices: notice::Bus,
    budgets: Arc<Mutex<HashMap<String, budget::Execution>>>,
    pressure: Arc<Mutex<()>>,
    closing: Arc<AtomicBool>,
    degraded: Arc<AtomicBool>,
    supervisor: Arc<dyn jobs::Supervisor>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub database: String,
    pub runtime: String,
    pub execution: String,
    pub bind: Option<String>,
    pub constitution: Option<String>,
}

pub struct Delivery<'a> {
    pub subscription: &'a str,
    pub id: &'a str,
    pub digest: &'a str,
}

pub struct Envelope<'a> {
    pub soul: &'a str,
    pub label: &'a str,
    pub text: String,
    pub source: Option<crate::ingest::Source>,
}

impl Service {
    pub fn open(config: Config, provider: Arc<dyn Provider>) -> Result<Self, String> {
        Self::supervised(config, provider, Arc::new(jobs::Unavailable))
    }

    pub fn supervised(
        config: Config,
        provider: Arc<dyn Provider>,
        supervisor: Arc<dyn jobs::Supervisor>,
    ) -> Result<Self, String> {
        let store = Store::open(&config.database)?;
        let context = plumb::context::Context::root().with(store.sink());
        {
            let _entered = context.enter();
            store.reconciled()?;
        }
        let degraded = store.strained()? > 0;
        Ok(Self {
            store,
            context,
            provider,
            config,
            materials: Arc::new(Mutex::new(HashMap::new())),
            streams: broadcast::channel(1024).0,
            errors: broadcast::channel(1024).0,
            notices: notice::Bus::new(),
            budgets: Arc::new(Mutex::new(HashMap::new())),
            pressure: Arc::new(Mutex::new(())),
            closing: Arc::new(AtomicBool::new(false)),
            degraded: Arc::new(AtomicBool::new(degraded)),
            supervisor,
        })
    }

    pub fn ration(&self, strand: &str, budget: budget::Execution) -> Result<(), String> {
        budget.validate()?;
        if self.store.strand(strand)?.is_none() {
            return Err("strand not found".to_string());
        }
        self.budgets
            .lock()
            .unwrap()
            .insert(strand.to_string(), budget);
        Ok(())
    }

    pub(in crate::service) fn rationed(&self, strand: &str) -> Option<budget::Execution> {
        self.budgets.lock().unwrap().get(strand).cloned()
    }

    pub fn close(&self) {
        self.closing.store(true, Ordering::SeqCst);
    }

    pub fn closing(&self) -> bool {
        self.closing.load(Ordering::SeqCst)
    }

    pub async fn drain(&self, cap: Duration) {
        let start = Instant::now();
        loop {
            let remaining = match self.store.running() {
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

    pub fn resume(&self) -> Result<(), String> {
        self.recover()?;
        self.dispatched();
        let pending = self.store.awaiting()?;
        for strand in pending {
            let outcome = self.poke(&strand, "strand_send", None, "cold_start_resume");
            if let drive::Outcome::Failed(error) = outcome
                && error.code == crate::catalog::UNSAVED.code
            {
                return Err(format!(
                    "cold-start recovery could not persist driver truth for strand {strand}: {}",
                    error.message
                ));
            }
        }
        Ok(())
    }

    pub fn degraded(&self) -> bool {
        self.degraded.load(Ordering::SeqCst)
    }

    pub fn strained(&self) -> i64 {
        match self.store.strained() {
            Ok(count) => count,
            Err(error) => {
                self.degraded.store(true, Ordering::SeqCst);
                eprintln!("santi: drive health count failed: {error}");
                0
            }
        }
    }

    pub(in crate::service) fn degrade(&self) {
        self.degraded.store(true, Ordering::SeqCst);
    }

    pub(in crate::service) fn refreshed(&self) {
        match self.store.strained() {
            Ok(count) => self.degraded.store(count > 0, Ordering::SeqCst),
            Err(error) => {
                self.degraded.store(true, Ordering::SeqCst);
                eprintln!("santi: drive health refresh failed: {error}");
            }
        }
    }

    pub fn listen(&self) -> broadcast::Receiver<stream::Event> {
        let receiver = self.streams.subscribe();
        self.dispatched();
        receiver
    }

    pub fn harken(&self) -> broadcast::Receiver<Transition> {
        let receiver = self.errors.subscribe();
        self.dispatched();
        receiver
    }
}
