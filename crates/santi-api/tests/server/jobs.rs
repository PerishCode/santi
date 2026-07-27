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
use sha2::{Digest, Sha256};

use super::*;

struct FakeSupervisor {
    launches: Mutex<usize>,
}

impl JobSupervisor for FakeSupervisor {
    fn ensure(&self, _launch: &JobLaunch) -> Result<(), String> {
        *self.launches.lock().unwrap() += 1;
        Ok(())
    }

    fn inspect(&self, _launch: &JobLaunch) -> Result<JobObservation, String> {
        Ok(JobObservation::Accepted)
    }

    fn stop(&self, _launch: &JobLaunch) -> Result<(), String> {
        Ok(())
    }

    fn acknowledge(&self, _launch: &JobLaunch) -> Result<(), String> {
        Ok(())
    }
}

#[tokio::test]
async fn accepts() {
    let temp = tempfile::tempdir().expect("temp dir");
    let database = temp.path().join("santi.sqlite");
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
        },
        Arc::new(DriverProvider),
        supervisor.clone(),
    )
    .expect("open service");
    let strand = service.weave().expect("create strand").strand;
    let capability = "jobcap_http_probe";
    let digest = format!("{:x}", Sha256::digest(capability.as_bytes()));
    let expiry = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis() as i64
        + 60_000;
    let conn = Connection::open(&database).expect("open sqlite");
    conn.execute(
        r#"
        INSERT INTO job_capabilities (
            digest, soul_id, strand_id, turn_id, tool_call_id, effect_id,
            expires_at, consumed_job_id, request_sha256, created_at
        )
        VALUES (?1, ?2, ?3, 'turn_http', 'call_http', 'effect_http',
                ?4, NULL, NULL, '2026-07-27T00:00:00.000Z')
        "#,
        rusqlite::params![digest, strand.soul, strand.id, expiry],
    )
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
    };

    let (status, Json(first)) =
        create_job_handler(State(service.clone()), headers.clone(), Json(request()))
            .await
            .unwrap_or_else(|error| panic!("create failed: {}", error.message()));
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(first.job.state, job::State::Accepted);
    assert_eq!(first.job.origin.soul, strand.soul);

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
