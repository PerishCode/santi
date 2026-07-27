use std::sync::{Arc, Condvar, Mutex, atomic::Ordering};

use rusqlite::Connection;
use santi_core::{
    job,
    service::{JobDraft, JobLaunch, JobObservation, JobSupervisor},
};

use super::{Probe, open, seed};

struct Gate {
    launch: Mutex<Option<JobLaunch>>,
    arrived: Condvar,
    open: Mutex<bool>,
    released: Condvar,
}

impl Gate {
    fn new() -> Self {
        Self {
            launch: Mutex::new(None),
            arrived: Condvar::new(),
            open: Mutex::new(false),
            released: Condvar::new(),
        }
    }

    fn launch(&self) -> JobLaunch {
        let mut launch = self.launch.lock().unwrap();
        while launch.is_none() {
            launch = self.arrived.wait(launch).unwrap();
        }
        launch.clone().expect("launch")
    }

    fn release(&self) {
        *self.open.lock().unwrap() = true;
        self.released.notify_all();
    }
}

impl JobSupervisor for Gate {
    fn detach(&self, launch: &JobLaunch) -> Result<(), String> {
        *self.launch.lock().unwrap() = Some(launch.clone());
        self.arrived.notify_all();
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self.released.wait(open).unwrap();
        }
        Ok(())
    }

    fn observe(&self, _launch: &JobLaunch) -> Result<JobObservation, String> {
        Ok(JobObservation::Missing)
    }

    fn stop(&self, _launch: &JobLaunch) -> Result<(), String> {
        Ok(())
    }

    fn acknowledge(&self, _launch: &JobLaunch) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn aborts() {
    let temp = tempfile::tempdir().expect("temp dir");
    let database = temp.path().join("santi.sqlite");
    let supervisor = Arc::new(Probe::new());
    let service = open(&temp, &database, supervisor.clone());
    let strand = service.weave().expect("create strand").strand;
    let capability = "jobcap_abort_probe";
    seed(&database, capability, &strand.soul, &strand.id);
    supervisor.refuse("detached handoff failed");

    let error = service
        .spawn(
            capability,
            JobDraft {
                description: "abandoned submission probe".to_string(),
                command: "true".to_string(),
                cwd: None,
                timeout: None,
                output: None,
                remind: None,
            },
        )
        .expect_err("pre-claim failure must reject create");
    assert_eq!(error, "detached handoff failed");
    let launch = supervisor
        .launches
        .lock()
        .unwrap()
        .first()
        .cloned()
        .expect("attempted launch");
    supervisor.set(JobObservation::Missing);
    let failed = service
        .job(&strand.soul, &launch.job.id)
        .expect("query abandoned job")
        .expect("job");
    assert_eq!(failed.state, job::State::Failed);
    assert_eq!(failed.reason.as_deref(), Some("submission_aborted"));
}

#[test]
fn resumes() {
    let temp = tempfile::tempdir().expect("temp dir");
    let database = temp.path().join("santi.sqlite");
    let supervisor = Arc::new(Probe::new());
    let service = open(&temp, &database, supervisor.clone());
    let strand = service.weave().expect("create strand").strand;
    let capability = "jobcap_resume_probe";
    seed(&database, capability, &strand.soul, &strand.id);
    let accepted = service
        .spawn(
            capability,
            JobDraft {
                description: "isolated cold start probe".to_string(),
                command: "sleep 30".to_string(),
                cwd: None,
                timeout: None,
                output: None,
                remind: None,
            },
        )
        .expect("accept job");
    drop(service);

    supervisor.fault("backend observation unavailable");
    let restarted = open(&temp, &database, supervisor.clone());
    restarted.resume().expect("resume runtime");
    assert_eq!(supervisor.observations.load(Ordering::SeqCst), 0);
    let listed = restarted.jobs(&strand.soul).expect("list jobs");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].state, job::State::Accepted);
    assert_eq!(supervisor.observations.load(Ordering::SeqCst), 1);
    let state: String = Connection::open(&database)
        .expect("open database")
        .query_row(
            "SELECT state FROM jobs WHERE id = ?1",
            [&accepted.job.id],
            |row| row.get(0),
        )
        .expect("stored job state");
    assert_eq!(state, "accepted");
}

#[test]
fn guards() {
    let temp = tempfile::tempdir().expect("temp dir");
    let database = temp.path().join("santi.sqlite");
    let gate = Arc::new(Gate::new());
    let service = open(&temp, &database, gate.clone());
    let strand = service.weave().expect("create strand").strand;
    let capability = "jobcap_guard_probe";
    seed(&database, capability, &strand.soul, &strand.id);
    let worker = {
        let service = service.clone();
        std::thread::spawn(move || {
            service.spawn(
                capability,
                JobDraft {
                    description: "handoff race probe".to_string(),
                    command: "true".to_string(),
                    cwd: None,
                    timeout: None,
                    output: None,
                    remind: None,
                },
            )
        })
    };
    let launch = gate.launch();
    let submitting = service
        .job(&strand.soul, &launch.job.id)
        .expect("query submitting job")
        .expect("job");
    assert_eq!(submitting.state, job::State::Submitting);
    gate.release();
    let accepted = worker.join().expect("join create").expect("accept job");
    assert_eq!(accepted.job.state, job::State::Accepted);
}
