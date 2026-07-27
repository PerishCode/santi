use std::{
    path::PathBuf,
    process::Command,
    sync::Arc,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use futures_util::stream;
use rusqlite::Connection;
use santi_core::{
    job,
    service::{self, JobLaunch, JobObservation, JobSupervisor, JobTerminal, Service},
};
use santi_provider::{Event, Metadata, Provider, Request, Streaming};
use sha2::{Digest, Sha256};

pub struct Silent;

#[async_trait]
impl Provider for Silent {
    fn metadata(&self) -> Metadata {
        Metadata {
            provider: Arc::from("job-test"),
            model: "job-test".to_string(),
            budget: None,
        }
    }

    async fn stream(&self, _request: Request) -> Result<Streaming, String> {
        Ok(Box::pin(stream::iter(vec![Ok(Event::Completed {
            response: None,
        })])))
    }
}

pub struct Guard<'a> {
    pub supervisor: &'a santi_api::jobs::Systemd,
    pub launch: &'a JobLaunch,
}

pub struct Unit {
    pub unit: String,
}

impl Drop for Unit {
    fn drop(&mut self) {
        let _ = Command::new("systemctl")
            .args(["--user", "stop", &self.unit])
            .status();
        let _ = Command::new("systemctl")
            .args(["--user", "reset-failed", &self.unit])
            .status();
    }
}

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        let _ = self.supervisor.acknowledge(self.launch);
    }
}

pub fn launch(
    temp: &tempfile::TempDir,
    description: &str,
    command: &str,
    output: u64,
) -> (santi_api::jobs::Systemd, JobLaunch) {
    let id = format!("job_{}", uuid::Uuid::new_v4().simple());
    let stamp = format!("stamp_{}", uuid::Uuid::new_v4().simple());
    let supervisor = format!("santi-{}.service", stamp.replace('_', "-"));
    (
        santi_api::jobs::Systemd::new(env!("CARGO_BIN_EXE_santi-api")),
        JobLaunch {
            job: job::Job {
                id: id.clone(),
                origin: job::Origin {
                    soul: "soul_probe".to_string(),
                    strand: "strand_probe".to_string(),
                    turn: "turn_probe".to_string(),
                    call: "call_probe".to_string(),
                    effect: "effect_probe".to_string(),
                },
                description: description.to_string(),
                command: command.to_string(),
                cwd: None,
                timeout_seconds: 30,
                output_limit_bytes: output,
                remind: None,
                state: job::State::Accepted,
                reason: None,
                exit_code: None,
                created: "2026-07-27T00:00:00.000Z".to_string(),
                updated: "2026-07-27T00:00:00.000Z".to_string(),
                accepted: Some("2026-07-27T00:00:00.000Z".to_string()),
                started: None,
                last: None,
                next: None,
                finished: None,
                acknowledged: None,
            },
            stamp,
            sidecar: supervisor,
            cwd: temp.path().display().to_string(),
            directory: temp.path().join("job").display().to_string(),
        },
    )
}

pub fn terminal(supervisor: &santi_api::jobs::Systemd, launch: &JobLaunch) -> JobTerminal {
    (0..100)
        .find_map(|_| {
            let observed = supervisor.observe(launch).expect("observe job");
            if let JobObservation::Terminal(terminal) = observed {
                Some(terminal)
            } else {
                thread::sleep(Duration::from_millis(50));
                None
            }
        })
        .expect("terminal evidence")
}

pub fn available() -> bool {
    Command::new("systemctl")
        .args(["--user", "show-environment"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub fn alive(pid: &str) -> bool {
    Command::new("kill")
        .args(["-0", pid])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub fn state(unit: &str) -> String {
    let output = Command::new("systemctl")
        .args(["--user", "show", unit, "--property=LoadState", "--value"])
        .output()
        .expect("inspect load state");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

pub fn seed(database: &std::path::Path, token: &str, soul: &str, strand: &str) {
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
        VALUES (?1, ?2, ?3, 'turn_vertical', 'call_vertical', 'effect_vertical',
                ?4, NULL, NULL, '2026-07-27T00:00:00.000Z')
        "#,
        rusqlite::params![digest, soul, strand, expiry],
    )
    .expect("seed capability");
}

pub fn stamp(database: &std::path::Path, job: &str) -> String {
    Connection::open(database)
        .expect("open sqlite")
        .query_row("SELECT generation FROM jobs WHERE id = ?1", [job], |row| {
            row.get(0)
        })
        .expect("job stamp")
}

pub fn service(temp: &tempfile::TempDir, database: &std::path::Path) -> Service {
    Service::supervised(
        service::Config {
            database: database.display().to_string(),
            runtime: temp.path().join("runtime").display().to_string(),
            execution: temp.path().join("execution").display().to_string(),
            bind: Some("127.0.0.1:0".to_string()),
            constitution: None,
        },
        Arc::new(Silent),
        Arc::new(santi_api::jobs::Systemd::new(env!(
            "CARGO_BIN_EXE_santi-api"
        ))),
    )
    .expect("open service")
}

pub fn path(launch: &JobLaunch) -> PathBuf {
    PathBuf::from(&launch.directory)
}
