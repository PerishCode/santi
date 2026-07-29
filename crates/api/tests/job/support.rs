#[cfg(unix)]
use std::process::Command;
use std::{
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use futures_util::stream;
use santi_core::{
    job,
    service::{self, JobLaunch, JobObservation, JobSupervisor, JobTerminal, Service},
};
use santi_estate::{CallDraft, CapabilityDraft, EffectDraft, TurnDraft};
use santi_provider::{Event, Metadata, Provider, Request, Streaming};
use sha2::{Digest, Sha256};

mod native;

pub use native::{alive, available, state};

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
    pub supervisor: &'a santi_api::jobs::Native,
    pub launch: &'a JobLaunch,
}

pub struct Unit {
    #[cfg(unix)]
    pub unit: String,
}

impl Drop for Unit {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        {
            let _ = Command::new("systemctl")
                .args(["--user", "stop", &self.unit])
                .status();
            let _ = Command::new("systemctl")
                .args(["--user", "reset-failed", &self.unit])
                .status();
        }
        #[cfg(target_os = "macos")]
        {
            let _ = Command::new("launchctl")
                .args(["bootout", &target(&self.unit)])
                .status();
        }
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
) -> (santi_api::jobs::Native, JobLaunch) {
    let id = format!("job_{}", uuid::Uuid::new_v4().simple());
    let stamp = format!("stamp_{}", uuid::Uuid::new_v4().simple());
    let supervisor = format!("santi-{}.service", stamp.replace('_', "-"));
    (
        santi_api::jobs::Native::new(env!("CARGO_BIN_EXE_santi-api")),
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

pub fn terminal(supervisor: &santi_api::jobs::Native, launch: &JobLaunch) -> JobTerminal {
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

pub async fn seed(database: &std::path::Path, token: &str, soul: &str, strand: &str) {
    let digest = format!("{:x}", Sha256::digest(token.as_bytes()));
    let expiry = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis() as i64
        + 60_000;
    let store = santi_core::Store::open(database)
        .await
        .expect("open estate");
    store
        .create_turn(TurnDraft {
            tag: "turn_vertical",
            strand,
            trigger: santi_core::turn::Trigger::System,
            source: None,
            from: 0,
            created: &santi_core::now(),
        })
        .await
        .expect("turn");
    store
        .create_call(CallDraft {
            tag: "call_vertical",
            turn: "turn_vertical",
            tool: "shell",
            arguments: &serde_json::json!({"command": "true"}),
            created: &santi_core::now(),
        })
        .await
        .expect("call");
    store
        .prepare_effect(EffectDraft {
            tag: "effect_vertical",
            turn: "turn_vertical",
            call: Some("call_vertical"),
            kind: "shell",
            metadata: None,
            created: &santi_core::now(),
        })
        .await
        .expect("effect");
    store
        .create_capability(CapabilityDraft {
            digest: &digest,
            expires: expiry,
            soul,
            strand,
            turn: "turn_vertical",
            call: "call_vertical",
            effect: "effect_vertical",
            created: &santi_core::now(),
        })
        .await
        .expect("seed capability");
}

pub async fn stamp(database: &std::path::Path, job: &str) -> String {
    santi_core::Store::open(database)
        .await
        .expect("open estate")
        .job_record(job)
        .await
        .expect("job query")
        .expect("job")
        .generation
}

pub async fn service(temp: &tempfile::TempDir, database: &std::path::Path) -> Service {
    Service::supervised(
        service::Config {
            database: database.display().to_string(),
            runtime: temp.path().join("runtime").display().to_string(),
            execution: temp.path().join("execution").display().to_string(),
            bind: Some("127.0.0.1:0".to_string()),
            constitution: None,
        },
        Arc::new(Silent),
        Arc::new(santi_api::jobs::Native::new(env!(
            "CARGO_BIN_EXE_santi-api"
        ))),
    )
    .await
    .expect("open service")
}

pub fn path(launch: &JobLaunch) -> PathBuf {
    PathBuf::from(&launch.directory)
}

#[cfg(target_os = "macos")]
fn domain() -> String {
    let output = Command::new("id").arg("-u").output().expect("inspect uid");
    format!("gui/{}", String::from_utf8_lossy(&output.stdout).trim())
}

#[cfg(target_os = "macos")]
fn target(unit: &str) -> String {
    format!("{}/{unit}", domain())
}
