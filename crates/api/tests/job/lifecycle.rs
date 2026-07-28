use std::{sync::Arc, thread, time::Duration};

use santi_core::{
    job,
    service::{self, JobDraft, JobRead, Service},
};

use super::support::{Silent, Unit, available, seed, service, stamp};

#[test]
fn vertical() {
    if !available() {
        eprintln!("skipping native job test: user supervisor is unavailable");
        return;
    }
    let temp = tempfile::tempdir().expect("temp dir");
    let database = temp.path().join("santi.sqlite");
    let supervisor = santi_api::jobs::Native::new(env!("CARGO_BIN_EXE_santi-api"));
    let service = Service::supervised(
        service::Config {
            database: database.display().to_string(),
            runtime: temp.path().join("runtime").display().to_string(),
            execution: temp.path().join("execution").display().to_string(),
            bind: Some("127.0.0.1:0".to_string()),
            constitution: None,
        },
        Arc::new(Silent),
        Arc::new(supervisor),
    )
    .expect("open service");
    let strand = service.weave().expect("create strand").strand;
    let capability = "jobcap_vertical_probe";
    seed(&database, capability, &strand.soul, &strand.id);

    let accepted = service
        .spawn(
            capability,
            JobDraft {
                description: "vertical slice probe".to_string(),
                command: "printf 'vertical-ok\\n'".to_string(),
                cwd: None,
                timeout: Some(30),
                output: Some(4096),
                remind: None,
            },
        )
        .expect("accept job");
    let stamp = stamp(&database, &accepted.job.id);
    let guard = Unit {
        unit: format!("santi-{}.service", stamp.replace('_', "-")),
    };
    assert_eq!(accepted.job.state, job::State::Accepted);
    let state: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            temp.path()
                .join("runtime")
                .join("jobs")
                .join(&stamp)
                .join("state.json"),
        )
        .expect("claimed state"),
    )
    .expect("decode state");
    assert_eq!(state["schema"], "santi.job.state.v1");
    assert!(
        matches!(
            state["phase"].as_str(),
            Some("claimed" | "running" | "terminal")
        ),
        "{state}"
    );
    assert!(
        !temp
            .path()
            .join("runtime")
            .join("jobs")
            .join(&accepted.job.id)
            .exists()
    );
    let completed = (0..100)
        .find_map(|_| {
            let current = service
                .job(&strand.soul, &accepted.job.id)
                .expect("query job")
                .expect("job");
            if current.state.terminal() {
                Some(current)
            } else {
                thread::sleep(Duration::from_millis(50));
                None
            }
        })
        .expect("job completion");
    assert_eq!(completed.state, job::State::Succeeded);
    let log = service
        .logs(JobRead {
            soul: &strand.soul,
            id: &accepted.job.id,
            stream: job::Stream::Stdout,
            cursor: "0",
            limit: 4096,
        })
        .expect("read log")
        .expect("log");
    assert_eq!(log.data, "vertical-ok\n");
    let acknowledged = service
        .ack(&strand.soul, &accepted.job.id)
        .expect("acknowledge")
        .expect("job");
    assert!(acknowledged.acknowledged.is_some());
    std::mem::forget(guard);
}

#[test]
fn restarts() {
    if !available() {
        eprintln!("skipping native job test: user supervisor is unavailable");
        return;
    }
    let temp = tempfile::tempdir().expect("temp dir");
    let database = temp.path().join("santi.sqlite");
    let runtime = service(&temp, &database);
    let strand = runtime.weave().expect("create strand").strand;
    let capability = "jobcap_restart_probe";
    seed(&database, capability, &strand.soul, &strand.id);
    let accepted = runtime
        .spawn(
            capability,
            JobDraft {
                description: "restart reconciliation probe".to_string(),
                command: "sleep 0.2; printf 'after-restart\\n'".to_string(),
                cwd: None,
                timeout: Some(30),
                output: Some(4096),
                remind: None,
            },
        )
        .expect("accept job");
    let stamp = stamp(&database, &accepted.job.id);
    let guard = Unit {
        unit: format!("santi-{}.service", stamp.replace('_', "-")),
    };
    drop(runtime);
    thread::sleep(Duration::from_millis(400));

    let restarted = service(&temp, &database);
    restarted.resume().expect("cold-start reconcile");
    let completed = (0..100)
        .find_map(|_| {
            let current = restarted
                .job(&strand.soul, &accepted.job.id)
                .expect("query")
                .expect("job");
            if current.state.terminal() {
                Some(current)
            } else {
                thread::sleep(Duration::from_millis(50));
                None
            }
        })
        .expect("reconciled completion");
    assert_eq!(completed.state, job::State::Succeeded);
    let log = restarted
        .logs(JobRead {
            soul: &strand.soul,
            id: &accepted.job.id,
            stream: job::Stream::Stdout,
            cursor: "0",
            limit: 4096,
        })
        .expect("read log")
        .expect("log");
    assert_eq!(log.data, "after-restart\n");
    restarted
        .ack(&strand.soul, &accepted.job.id)
        .expect("ack")
        .expect("job");
    std::mem::forget(guard);
}
