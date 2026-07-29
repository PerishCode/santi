use std::{sync::Arc, time::Duration};

use santi_core::{
    job,
    service::{self, JobDraft, JobRead, Service},
};

use super::support::{Silent, Unit, available, seed, service, stamp};

#[tokio::test]
async fn vertical() {
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
            environment: Default::default(),
        },
        Arc::new(Silent),
        Arc::new(supervisor),
    )
    .await
    .expect("open service");
    let strand = service.weave().await.expect("create strand").strand;
    let capability = "jobcap_vertical_probe";
    seed(&database, capability, &strand.soul, &strand.id).await;

    let accepted = service
        .spawn(
            capability,
            JobDraft {
                description: "vertical slice probe".to_string(),
                command: vertical_command().to_string(),
                cwd: None,
                timeout: Some(30),
                output: Some(4096),
                remind: None,
            },
        )
        .await
        .expect("accept job");
    let stamp = stamp(&database, &accepted.job.id).await;
    let guard = Unit {
        #[cfg(unix)]
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
    let mut completed = None;
    for _ in 0..100 {
        let current = service
            .job(&strand.soul, &accepted.job.id)
            .await
            .expect("query job")
            .expect("job");
        if current.state.terminal() {
            completed = Some(current);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let completed = completed.expect("job completion");
    assert_eq!(completed.state, job::State::Succeeded);
    let log = service
        .logs(JobRead {
            soul: &strand.soul,
            id: &accepted.job.id,
            stream: job::Stream::Stdout,
            cursor: "0",
            limit: 4096,
        })
        .await
        .expect("read log")
        .expect("log");
    #[cfg(unix)]
    assert_eq!(log.data, "vertical-ok\n");
    #[cfg(target_os = "windows")]
    assert_eq!(log.data, "vertical-ok\r\n");
    let acknowledged = service
        .ack(&strand.soul, &accepted.job.id)
        .await
        .expect("acknowledge")
        .expect("job");
    assert!(acknowledged.acknowledged.is_some());
    std::mem::forget(guard);
}

#[tokio::test]
async fn restarts() {
    if !available() {
        eprintln!("skipping native job test: user supervisor is unavailable");
        return;
    }
    let temp = tempfile::tempdir().expect("temp dir");
    let database = temp.path().join("santi.sqlite");
    let runtime = service(&temp, &database).await;
    let strand = runtime.weave().await.expect("create strand").strand;
    let capability = "jobcap_restart_probe";
    seed(&database, capability, &strand.soul, &strand.id).await;
    let accepted = runtime
        .spawn(
            capability,
            JobDraft {
                description: "restart reconciliation probe".to_string(),
                command: restart_command().to_string(),
                cwd: None,
                timeout: Some(30),
                output: Some(4096),
                remind: None,
            },
        )
        .await
        .expect("accept job");
    #[cfg(unix)]
    let stamp = stamp(&database, &accepted.job.id).await;
    let guard = Unit {
        #[cfg(unix)]
        unit: format!("santi-{}.service", stamp.replace('_', "-")),
    };
    drop(runtime);
    tokio::time::sleep(Duration::from_millis(400)).await;

    let restarted = service(&temp, &database).await;
    restarted.resume().await.expect("cold-start reconcile");
    let mut completed = None;
    for _ in 0..100 {
        let current = restarted
            .job(&strand.soul, &accepted.job.id)
            .await
            .expect("query")
            .expect("job");
        if current.state.terminal() {
            completed = Some(current);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let completed = completed.expect("reconciled completion");
    assert_eq!(completed.state, job::State::Succeeded);
    let log = restarted
        .logs(JobRead {
            soul: &strand.soul,
            id: &accepted.job.id,
            stream: job::Stream::Stdout,
            cursor: "0",
            limit: 4096,
        })
        .await
        .expect("read log")
        .expect("log");
    #[cfg(unix)]
    assert_eq!(log.data, "after-restart\n");
    #[cfg(target_os = "windows")]
    assert_eq!(log.data, "after-restart\r\n");
    restarted
        .ack(&strand.soul, &accepted.job.id)
        .await
        .expect("ack")
        .expect("job");
    std::mem::forget(guard);
}

#[cfg(unix)]
fn vertical_command() -> &'static str {
    "printf 'vertical-ok\\n'"
}

#[cfg(target_os = "windows")]
fn vertical_command() -> &'static str {
    r#"Write-Output "vertical-ok""#
}

#[cfg(unix)]
fn restart_command() -> &'static str {
    "sleep 0.2; printf 'after-restart\\n'"
}

#[cfg(target_os = "windows")]
fn restart_command() -> &'static str {
    r#"Start-Sleep -Milliseconds 200; Write-Output "after-restart""#
}
