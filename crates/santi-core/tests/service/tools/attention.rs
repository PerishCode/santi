use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::Connection;
use santi_core::{
    job,
    service::{self, JobDraft, JobLaunch, JobObservation, JobSupervisor, Service},
};
use sha2::{Digest, Sha256};

use crate::support::{FakeProvider, GatedFirstProvider, Probe};

struct Running;

impl JobSupervisor for Running {
    fn detach(&self, _launch: &JobLaunch) -> Result<(), String> {
        Ok(())
    }

    fn observe(&self, _launch: &JobLaunch) -> Result<JobObservation, String> {
        Ok(JobObservation::Running)
    }

    fn stop(&self, _launch: &JobLaunch) -> Result<(), String> {
        Ok(())
    }

    fn acknowledge(&self, _launch: &JobLaunch) -> Result<(), String> {
        Ok(())
    }
}

#[tokio::test]
async fn aggregates() {
    let temp = tempfile::tempdir().expect("temp dir");
    let database = temp.path().join("santi.sqlite");
    let provider = FakeProvider::default();
    let service = open(&temp, &database, Arc::new(provider.clone()));
    let strand = service.weave().expect("create strand").strand;
    let first = create(&service, &database, &strand, "one");
    let second = create(&service, &database, &strand, "two");
    age(&database, &[&first.id, &second.id], 35_000);

    let watcher = tokio::spawn({
        let service = service.clone();
        async move { service.watch().await }
    });
    let runtime = Probe::new(&service)
        .message_containing(&strand.id, "kind: inbox_attention")
        .await;
    service.close();
    watcher.await.expect("join watcher");

    let messages = runtime
        .messages
        .iter()
        .filter(|message| message.text.contains("kind: inbox_attention"))
        .collect::<Vec<_>>();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].text.contains("items: 2"));
    assert!(messages[0].text.contains(&first.id));
    assert!(messages[0].text.contains(&second.id));
    assert_eq!(
        messages[0].text.matches("causes: [\"reminder\"]").count(),
        2
    );
    assert_eq!(provider.requests.lock().unwrap().len(), 1);

    let conn = Connection::open(&database).expect("open sqlite");
    for id in [&first.id, &second.id] {
        let state: (i64, i64, Option<String>, Option<String>) = conn
            .query_row(
                r#"
                SELECT attention_revision, reminder_tick,
                       last_reminded_at, next_reminder_at
                FROM jobs WHERE id = ?1
                "#,
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("attention state");
        assert_eq!(state.0, 1);
        assert_eq!(state.1, 3);
        assert!(state.2.is_some());
        assert!(state.3.is_some());
    }
}

#[tokio::test]
async fn coalesces() {
    let temp = tempfile::tempdir().expect("temp dir");
    let database = temp.path().join("santi.sqlite");
    let provider = GatedFirstProvider::new();
    let service = open(&temp, &database, Arc::new(provider.clone()));
    let strand = service.weave().expect("create strand").strand;
    let job = create(&service, &database, &strand, "held");
    age(&database, &[&job.id], 25_000);

    let watcher = tokio::spawn({
        let service = service.clone();
        async move { service.watch().await }
    });
    provider.wait_for_first_request().await;
    write(&temp, &database, &job.id, 90);
    wait(&database, &job.id, 2).await;
    age(&database, &[&job.id], 250_000);
    wait(&database, &job.id, 3).await;

    let conn = Connection::open(&database).expect("open sqlite");
    let receipts: i64 = conn
        .query_row("SELECT COUNT(*) FROM inbox_receipts", [], |row| row.get(0))
        .expect("receipt count");
    assert_eq!(
        receipts, 2,
        "pending updates must retain one responsibility"
    );
    drop(conn);

    provider.release_first_request();
    let runtime = Probe::new(&service).completed_count(&strand.id, 2).await;
    service.close();
    watcher.await.expect("join watcher");
    let messages = runtime
        .messages
        .iter()
        .filter(|message| message.text.contains("kind: inbox_attention"))
        .collect::<Vec<_>>();
    assert_eq!(messages.len(), 2);
    let latest = messages.last().expect("latest attention");
    assert!(latest.text.contains("output_threshold"));
    assert!(latest.text.contains("runtime_threshold"));
}

fn open(
    temp: &tempfile::TempDir,
    database: &std::path::Path,
    provider: Arc<dyn santi_provider::Provider>,
) -> Service {
    Service::supervised(
        service::Config {
            database: database.display().to_string(),
            runtime: temp.path().join("runtime").display().to_string(),
            execution: temp.path().join("execution").display().to_string(),
            bind: Some("127.0.0.1:0".to_string()),
            constitution: None,
        },
        provider,
        Arc::new(Running),
    )
    .expect("open service")
}

fn create(
    service: &Service,
    database: &std::path::Path,
    strand: &santi_core::strand::Strand,
    suffix: &str,
) -> job::Job {
    let token = format!("jobcap_attention_{suffix}");
    seed(database, &token, &strand.soul, &strand.id);
    let accepted = service
        .spawn(
            &token,
            JobDraft {
                description: format!("attention {suffix}"),
                command: "sleep 300".to_string(),
                cwd: None,
                timeout: Some(300),
                output: Some(100),
                remind: Some(10),
            },
        )
        .expect("accept job");
    service
        .job(&strand.soul, &accepted.job.id)
        .expect("refresh job")
        .expect("job")
}

fn seed(database: &std::path::Path, token: &str, soul: &str, strand: &str) {
    let digest = format!("{:x}", Sha256::digest(token.as_bytes()));
    let expiry = epoch() + 60_000;
    Connection::open(database)
        .expect("open sqlite")
        .execute(
            r#"
            INSERT INTO job_capabilities (
                digest, soul_id, strand_id, turn_id, tool_call_id, effect_id,
                expires_at, consumed_job_id, request_sha256, created_at
            )
            VALUES (?1, ?2, ?3, 'turn_attention', 'call_attention',
                    'effect_attention', ?4, NULL, NULL,
                    '2026-07-27T00:00:00.000Z')
            "#,
            rusqlite::params![digest, soul, strand, expiry],
        )
        .expect("seed capability");
}

fn age(database: &std::path::Path, jobs: &[&str], millis: i64) {
    let conn = Connection::open(database).expect("open sqlite");
    for job in jobs {
        conn.execute(
            "UPDATE jobs SET started_millis = ?2 WHERE id = ?1",
            rusqlite::params![job, epoch() - millis],
        )
        .expect("age job");
    }
}

fn write(temp: &tempfile::TempDir, database: &std::path::Path, job: &str, bytes: usize) {
    let stamp: String = Connection::open(database)
        .expect("open sqlite")
        .query_row("SELECT generation FROM jobs WHERE id = ?1", [job], |row| {
            row.get(0)
        })
        .expect("job stamp");
    let directory = temp.path().join("runtime").join("jobs").join(stamp);
    std::fs::create_dir_all(&directory).expect("create job directory");
    std::fs::write(directory.join("stdout.log"), vec![b'x'; bytes]).expect("write job output");
}

async fn wait(database: &std::path::Path, job: &str, revision: i64) {
    for _ in 0..100 {
        let current: i64 = Connection::open(database)
            .expect("open sqlite")
            .query_row(
                "SELECT attention_revision FROM jobs WHERE id = ?1",
                [job],
                |row| row.get(0),
            )
            .expect("attention revision");
        if current >= revision {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("attention revision {revision} did not arrive");
}

fn epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis() as i64
}
