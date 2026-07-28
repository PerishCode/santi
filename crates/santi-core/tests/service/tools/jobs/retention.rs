use std::{path::PathBuf, sync::Arc, time::Duration};

use rusqlite::Connection;
use santi_core::{
    job,
    service::{JobDraft, JobObservation, JobTerminal, Service},
};

use super::{Probe, open, seed};

#[tokio::test]
async fn reaps() {
    let temp = tempfile::tempdir().expect("temp dir");
    let database = temp.path().join("santi.sqlite");
    let supervisor = Arc::new(Probe::new());
    let service = open(&temp, &database, supervisor.clone())
        .retain(Duration::from_secs(60 * 60))
        .expect("set retention");
    let strand = service.weave().expect("create strand").strand;
    let old = create(&service, &database, &strand, "old");
    let young = create(&service, &database, &strand, "young");
    let unacked = create(&service, &database, &strand, "unacked");
    supervisor.set(JobObservation::Terminal(JobTerminal {
        state: job::State::Succeeded,
        reason: None,
        exit: Some(0),
    }));
    for id in [&old, &young, &unacked] {
        service
            .job(&strand.soul, id)
            .expect("refresh")
            .expect("job");
    }
    service
        .ack(&strand.soul, &old)
        .expect("ack old")
        .expect("old job");
    service
        .ack(&strand.soul, &young)
        .expect("ack young")
        .expect("young job");
    let conn = Connection::open(&database).expect("open database");
    conn.execute(
        "UPDATE jobs SET acknowledged_at = '2000-01-01T00:00:00.000Z' WHERE id = ?1",
        [&old],
    )
    .expect("age old job");
    drop(conn);
    let oldpath = artifact(&temp, &database, &old);
    let youngpath = artifact(&temp, &database, &young);
    let idlepath = artifact(&temp, &database, &unacked);

    let watcher = tokio::spawn({
        let service = service.clone();
        async move { service.watch().await }
    });
    for _ in 0..100 {
        if service
            .job(&strand.soul, &old)
            .expect("query old")
            .is_none()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    service.close();
    watcher.await.expect("join watcher");

    assert!(
        service
            .job(&strand.soul, &old)
            .expect("query old")
            .is_none(),
        "expired acknowledged job survived collection"
    );
    assert!(!oldpath.exists(), "expired artifacts survived collection");
    assert!(youngpath.is_dir());
    assert!(idlepath.is_dir());
    assert!(
        service
            .job(&strand.soul, &young)
            .expect("query young")
            .is_some()
    );
    assert!(
        service
            .job(&strand.soul, &unacked)
            .expect("query unacked")
            .is_some()
    );
    assert!(
        service.strand(&strand.id).expect("query strand").is_some(),
        "job collection removed strand history"
    );
    let conn = Connection::open(&database).expect("open database");
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM job_capabilities WHERE consumed_job_id = ?1",
            [&old],
            |row| row.get(0),
        )
        .expect("count old capabilities");
    assert_eq!(count, 0);
}

#[tokio::test]
async fn recovers() {
    let temp = tempfile::tempdir().expect("temp dir");
    let database = temp.path().join("santi.sqlite");
    let supervisor = Arc::new(Probe::new());
    let service = open(&temp, &database, supervisor.clone())
        .retain(Duration::from_secs(60 * 60))
        .expect("set retention");
    let strand = service.weave().expect("create strand").strand;
    let retained = create(&service, &database, &strand, "retained");
    supervisor.set(JobObservation::Terminal(JobTerminal {
        state: job::State::Succeeded,
        reason: None,
        exit: Some(0),
    }));
    service
        .job(&strand.soul, &retained)
        .expect("refresh")
        .expect("job");
    service
        .ack(&strand.soul, &retained)
        .expect("ack")
        .expect("job");
    let canonical = artifact(&temp, &database, &retained);
    let trash = temp.path().join("runtime").join("jobs").join(".gc");
    std::fs::create_dir_all(&trash).expect("create trash");
    let archived = trash.join(canonical.file_name().expect("artifact key"));
    std::fs::rename(&canonical, &archived).expect("simulate pre-commit crash");
    let orphan = trash.join("stamp_orphan");
    std::fs::create_dir(&orphan).expect("create orphan");

    let watcher = tokio::spawn({
        let service = service.clone();
        async move { service.watch().await }
    });
    for _ in 0..100 {
        if canonical.is_dir() && !orphan.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    service.close();
    watcher.await.expect("join watcher");

    assert!(canonical.is_dir(), "retained artifact was not restored");
    assert!(!archived.exists());
    assert!(!orphan.exists(), "committed orphan was not removed");
    assert!(
        service
            .job(&strand.soul, &retained)
            .expect("query retained")
            .is_some()
    );
}

fn create(
    service: &Service,
    database: &std::path::Path,
    strand: &santi_core::strand::Strand,
    suffix: &str,
) -> String {
    let capability = format!("jobcap_retention_{suffix}");
    seed(database, &capability, &strand.soul, &strand.id);
    service
        .spawn(
            &capability,
            JobDraft {
                description: format!("retention {suffix}"),
                command: "true".to_string(),
                cwd: None,
                timeout: None,
                output: None,
                remind: None,
            },
        )
        .expect("create job")
        .job
        .id
}

fn artifact(temp: &tempfile::TempDir, database: &std::path::Path, job: &str) -> PathBuf {
    let conn = Connection::open(database).expect("open database");
    let stamp: String = conn
        .query_row("SELECT generation FROM jobs WHERE id = ?1", [job], |row| {
            row.get(0)
        })
        .expect("job stamp");
    let path = temp.path().join("runtime").join("jobs").join(stamp);
    std::fs::create_dir_all(&path).expect("create artifact");
    std::fs::write(path.join("result.json"), b"{}").expect("write artifact");
    path
}
