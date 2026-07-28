use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::Connection;
use santi_core::{
    job,
    service::{self, JobDraft, JobLaunch, JobObservation, JobSupervisor, JobTerminal, Service},
};
use sha2::{Digest, Sha256};

use crate::support::FakeProvider;

mod cold;
mod retention;

struct Probe {
    observation: Mutex<JobObservation>,
    launches: Mutex<Vec<JobLaunch>>,
    acknowledgements: Mutex<usize>,
    failure: Mutex<Option<String>>,
    fault: Mutex<Option<String>>,
    observations: AtomicUsize,
}

impl Probe {
    fn new() -> Self {
        Self {
            observation: Mutex::new(JobObservation::Claimed),
            launches: Mutex::new(Vec::new()),
            acknowledgements: Mutex::new(0),
            failure: Mutex::new(None),
            fault: Mutex::new(None),
            observations: AtomicUsize::new(0),
        }
    }

    fn set(&self, observation: JobObservation) {
        *self.observation.lock().unwrap() = observation;
    }

    fn refuse(&self, error: &str) {
        *self.failure.lock().unwrap() = Some(error.to_string());
    }

    fn fault(&self, error: &str) {
        *self.fault.lock().unwrap() = Some(error.to_string());
    }
}

impl JobSupervisor for Probe {
    fn detach(&self, launch: &JobLaunch) -> Result<(), String> {
        self.launches.lock().unwrap().push(launch.clone());
        self.failure.lock().unwrap().clone().map_or(Ok(()), Err)
    }

    fn observe(&self, _launch: &JobLaunch) -> Result<JobObservation, String> {
        self.observations.fetch_add(1, Ordering::SeqCst);
        if let Some(error) = self.fault.lock().unwrap().clone() {
            return Err(error);
        }
        Ok(self.observation.lock().unwrap().clone())
    }

    fn stop(&self, _launch: &JobLaunch) -> Result<(), String> {
        self.set(JobObservation::Terminal(JobTerminal {
            state: job::State::Cancelled,
            reason: Some("cancel_requested".to_string()),
            exit: None,
        }));
        Ok(())
    }

    fn acknowledge(&self, _launch: &JobLaunch) -> Result<(), String> {
        *self.acknowledgements.lock().unwrap() += 1;
        Ok(())
    }
}

#[test]
fn accepts() {
    let temp = tempfile::tempdir().expect("temp dir");
    let database = temp.path().join("santi.sqlite");
    let supervisor = Arc::new(Probe::new());
    let service = open(&temp, &database, supervisor.clone());
    let strand = service.weave().expect("create strand").strand;
    let capability = "jobcap_core_probe";
    seed(&database, capability, &strand.soul, &strand.id);
    let draft = || JobDraft {
        description: "compile release".to_string(),
        command: "cargo build --release".to_string(),
        cwd: None,
        timeout: Some(90),
        output: Some(1024),
        remind: None,
    };

    let accepted = service.spawn(capability, draft()).expect("accept job");
    assert_eq!(accepted.job.state, job::State::Accepted);
    assert_eq!(accepted.job.origin.soul, strand.soul);
    assert_eq!(supervisor.launches.lock().unwrap().len(), 1);

    let retried = service
        .spawn(capability, draft())
        .expect("idempotent retry");
    assert_eq!(retried.job.id, accepted.job.id);
    assert_eq!(supervisor.launches.lock().unwrap().len(), 1);

    let error = service
        .spawn(
            capability,
            JobDraft {
                command: "cargo test".to_string(),
                ..draft()
            },
        )
        .expect_err("changed retry must conflict");
    assert!(error.contains("conflicts with its accepted request"));

    let error = service
        .spawn(
            "unused",
            JobDraft {
                remind: Some(0),
                ..draft()
            },
        )
        .expect_err("zero reminder must be rejected");
    assert!(error.contains("reminder interval must be greater than zero"));

    supervisor.set(JobObservation::Running);
    let running = service
        .job(&strand.soul, &accepted.job.id)
        .expect("get job")
        .expect("job");
    assert_eq!(running.state, job::State::Running);

    let cancelled = service
        .cancel(&strand.soul, &accepted.job.id)
        .expect("cancel")
        .expect("job");
    assert_eq!(cancelled.state, job::State::Cancelled);
    assert_eq!(cancelled.reason.as_deref(), Some("cancel_requested"));

    let acknowledged = service
        .ack(&strand.soul, &accepted.job.id)
        .expect("ack")
        .expect("job");
    assert!(acknowledged.acknowledged.is_some());
    assert_eq!(*supervisor.acknowledgements.lock().unwrap(), 1);
}

#[test]
fn abstains() {
    let temp = tempfile::tempdir().expect("temp dir");
    let database = temp.path().join("santi.sqlite");
    let supervisor = Arc::new(Probe::new());
    let service = open(&temp, &database, supervisor.clone());
    let strand = service.weave().expect("create strand").strand;
    let capability = "jobcap_unknown_probe";
    seed(&database, capability, &strand.soul, &strand.id);
    let accepted = service
        .spawn(
            capability,
            JobDraft {
                description: "cold start probe".to_string(),
                command: "true".to_string(),
                cwd: None,
                timeout: None,
                output: None,
                remind: None,
            },
        )
        .expect("accept job");
    supervisor.set(JobObservation::Missing);

    let unknown = service
        .job(&strand.soul, &accepted.job.id)
        .expect("get")
        .expect("job");
    assert_eq!(unknown.state, job::State::Unknown);
    assert_eq!(unknown.reason.as_deref(), Some("sidecar_evidence_missing"));
    assert_eq!(
        supervisor.launches.lock().unwrap().len(),
        1,
        "reconciliation must not replay the command"
    );
}

fn open(
    temp: &tempfile::TempDir,
    database: &std::path::Path,
    supervisor: Arc<dyn JobSupervisor>,
) -> Service {
    Service::supervised(
        service::Config {
            database: database.display().to_string(),
            runtime: temp.path().join("runtime").display().to_string(),
            execution: temp.path().join("execution").display().to_string(),
            bind: Some("127.0.0.1:0".to_string()),
            constitution: None,
        },
        Arc::new(FakeProvider::default()),
        supervisor,
    )
    .expect("open service")
}

fn seed(database: &std::path::Path, token: &str, soul: &str, strand: &str) {
    let digest = format!("{:x}", Sha256::digest(token.as_bytes()));
    let expiry = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis() as i64
        + 60_000;
    let conn = Connection::open(database).expect("open sqlite");
    conn.execute(
        r#"
        INSERT INTO job_capabilities (
            digest, soul_id, strand_id, turn_id, tool_call_id, effect_id,
            expires_at, consumed_job_id, request_sha256, created_at
        )
        VALUES (?1, ?2, ?3, 'turn_core', 'call_core', 'effect_core',
                ?4, NULL, NULL, '2026-07-27T00:00:00.000Z')
        "#,
        rusqlite::params![digest, soul, strand, expiry],
    )
    .expect("seed capability");
}
