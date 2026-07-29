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
mod interrupt;
mod text;
mod tools;
mod traces;

use santi_provider::Provider;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::broadcast;

use crate::{Ruled, Transition};
use crate::{budget, material, stream};
use santi_estate::Store;

pub const RETENTION: u64 = 7 * 24 * 60 * 60;

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
    inboxes: Arc<Mutex<HashMap<String, String>>>,
    budgets: Arc<Mutex<HashMap<String, budget::Execution>>>,
    pressure: Arc<tokio::sync::Mutex<()>>,
    closing: Arc<AtomicBool>,
    controls: Arc<Mutex<HashMap<String, interrupt::Control>>>,
    deadline: Arc<Mutex<Option<Instant>>>,
    degraded: Arc<AtomicBool>,
    supervisor: Arc<dyn jobs::Supervisor>,
    handoffs: Arc<Mutex<HashSet<String>>>,
    retention: Duration,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub database: String,
    pub runtime: String,
    pub execution: String,
    pub bind: Option<String>,
    pub constitution: Option<String>,
    pub environment: BTreeMap<String, String>,
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
    pub async fn open(config: Config, provider: Arc<dyn Provider>) -> Result<Self, String> {
        Self::supervised(config, provider, Arc::new(jobs::Unavailable)).await
    }

    pub async fn supervised(
        config: Config,
        provider: Arc<dyn Provider>,
        supervisor: Arc<dyn jobs::Supervisor>,
    ) -> Result<Self, String> {
        crate::environment::validate(&config.environment)?;
        let store = Store::open(&config.database).await?;
        store.seed(crate::GENESIS, &crate::now()).await?;
        let traces = traces::Writer::start(store.clone());
        let sink = plumb::trace::Sink::from(traces);
        let context = plumb::context::Context::root().with(sink);
        store
            .recover_turns("santi.cold_start", &crate::now())
            .await?;
        let degraded = store
            .active_incident_count(crate::drive::Error::Failed.descriptor().code)
            .await?
            > 0;
        Ok(Self {
            store,
            context,
            provider,
            config,
            materials: Arc::new(Mutex::new(HashMap::new())),
            streams: broadcast::channel(1024).0,
            errors: broadcast::channel(1024).0,
            notices: notice::Bus::new(),
            inboxes: Arc::new(Mutex::new(HashMap::new())),
            budgets: Arc::new(Mutex::new(HashMap::new())),
            pressure: Arc::new(tokio::sync::Mutex::new(())),
            closing: Arc::new(AtomicBool::new(false)),
            controls: Arc::new(Mutex::new(HashMap::new())),
            deadline: Arc::new(Mutex::new(None)),
            degraded: Arc::new(AtomicBool::new(degraded)),
            supervisor,
            handoffs: Arc::new(Mutex::new(HashSet::new())),
            retention: Duration::from_secs(RETENTION),
        })
    }

    pub async fn ration(&self, strand: &str, budget: budget::Execution) -> Result<(), String> {
        budget.validate()?;
        if self.store.strand(strand).await?.is_none() {
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

    pub async fn resume(&self) -> Result<(), String> {
        self.dispatched().await;
        let pending = self.store.pending_strands().await?;
        for strand in pending {
            let outcome = self
                .poke(&strand, "strand_send", None, "cold_start_resume")
                .await;
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

    pub async fn strained(&self) -> usize {
        match self
            .store
            .active_incident_count(crate::drive::Error::Failed.descriptor().code)
            .await
        {
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

    pub(in crate::service) async fn refreshed(&self) {
        match self
            .store
            .active_incident_count(crate::drive::Error::Failed.descriptor().code)
            .await
        {
            Ok(count) => self.degraded.store(count > 0, Ordering::SeqCst),
            Err(error) => {
                self.degraded.store(true, Ordering::SeqCst);
                eprintln!("santi: drive health refresh failed: {error}");
            }
        }
    }

    pub async fn listen(&self) -> broadcast::Receiver<stream::Event> {
        let receiver = self.streams.subscribe();
        self.dispatched().await;
        receiver
    }

    pub async fn harken(&self) -> broadcast::Receiver<Transition> {
        let receiver = self.errors.subscribe();
        self.dispatched().await;
        receiver
    }
}
