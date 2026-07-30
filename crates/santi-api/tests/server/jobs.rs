use std::{
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::http::HeaderMap;
use santi_api::{CreateJobRequest, create_job_handler, get_job_handler};
use santi_core::{
    job,
    service::{self, JobLaunch, JobObservation, JobSupervisor, Service},
};
use santi_estate::{CallDraft, CapabilityDraft, EffectDraft, TurnDraft};
use sha2::{Digest, Sha256};

use super::*;

struct FakeSupervisor {
    launches: Mutex<usize>,
}

impl JobSupervisor for FakeSupervisor {
    fn detach(&self, _: &JobLaunch) -> Result<(), String> {
        *self.launches.lock().unwrap() += 1;
        Ok(())
    }

    fn observe(&self, _: &JobLaunch) -> Result<JobObservation, String> {
        Ok(JobObservation::Claimed)
    }

    fn stop(&self, _: &JobLaunch) -> Result<(), String> {
        Ok(())
    }

    fn acknowledge(&self, _: &JobLaunch) -> Result<(), String> {
        Ok(())
    }
}

#[tokio::test]
async fn accepts() {
    let temp = tempfile::tempdir().expect("temp dir");
    let database = temp.path().join("santi.sqlite");
    super::support::bootstrap(&database).await;
    let supervisor = Arc::new(FakeSupervisor {
        launches: Mutex::new(0),
    });
    let service = Service::supervised(
        service::Config {
            database: database.display().to_string(),
            runtime: temp.path().join("runtime").display().to_string(),
            execution: temp.path().join("execution").display().to_string(),
            bind: Some("127.0.0.1:0".to_string()),
            constitution: None,
            environment: Default::default(),
        },
        Arc::new(DriverProvider),
        supervisor.clone(),
    )
    .await
    .expect("open service");
    let strand = service.weave().await.expect("create strand").strand;
    let capability = "jobcap_http_probe";
    let digest = format!("{:x}", Sha256::digest(capability.as_bytes()));
    let expiry = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis() as i64
        + 60_000;
    let store = santi_core::Store::open(&database)
        .await
        .expect("open estate");
    store
        .create_turn(TurnDraft {
            tag: "turn_http",
            strand: &strand.id,
            trigger: santi_core::turn::Trigger::System,
            source: None,
            from: 0,
            created: &santi_core::now(),
        })
        .await
        .expect("turn");
    store
        .create_call(CallDraft {
            tag: "call_http",
            turn: "turn_http",
            tool: "shell",
            arguments: &serde_json::json!({"command": "printf ok"}),
            created: &santi_core::now(),
        })
        .await
        .expect("call");
    store
        .prepare_effect(EffectDraft {
            tag: "effect_http",
            turn: "turn_http",
            call: Some("call_http"),
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
            soul: &strand.soul,
            strand: &strand.id,
            turn: "turn_http",
            call: "call_http",
            effect: "effect_http",
            created: &santi_core::now(),
        })
        .await
        .expect("seed capability");
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-santi-job-capability",
        capability.parse().expect("capability header"),
    );
    let request = || CreateJobRequest {
        description: "http boundary probe".to_string(),
        command: "printf ok".to_string(),
        cwd: None,
        timeout_seconds: Some(30),
        output_limit_bytes: Some(4096),
        remind_every_seconds: Some(5),
    };

    let (status, Json(first)) =
        create_job_handler(State(service.clone()), headers.clone(), Json(request()))
            .await
            .unwrap_or_else(|error| panic!("create failed: {}", error.message()));
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(first.job.state, job::State::Accepted);
    assert_eq!(first.job.origin.soul, strand.soul);
    assert_eq!(first.job.remind, Some(5));

    let (_, Json(retried)) = create_job_handler(State(service.clone()), headers, Json(request()))
        .await
        .unwrap_or_else(|error| panic!("retry failed: {}", error.message()));
    assert_eq!(retried.job.id, first.job.id);
    assert_eq!(*supervisor.launches.lock().unwrap(), 1);

    let mut owner = HeaderMap::new();
    owner.insert("x-santi-soul-id", strand.soul.parse().expect("soul header"));
    let Json(queried) = get_job_handler(State(service), owner, Path(first.job.id.clone()))
        .await
        .unwrap_or_else(|error| panic!("get failed: {}", error.message()));
    assert_eq!(queried.id, first.job.id);
}
