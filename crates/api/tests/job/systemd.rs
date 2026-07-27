use std::{thread, time::Duration};

use santi_core::{
    job,
    service::{JobLaunch, JobSupervisor},
};

use super::support::{Guard, alive, available, launch, path, state, terminal};

#[test]
fn retains() {
    if !available() {
        eprintln!("skipping systemd job test: user manager is unavailable");
        return;
    }
    let temp = tempfile::tempdir().expect("temp dir");
    let id = format!("job_{}", uuid::Uuid::new_v4().simple());
    let supervisor_ref = format!("santi-{}.service", id.replace('_', "-"));
    let supervisor = santi_api::jobs::Systemd::new(env!("CARGO_BIN_EXE_santi-api"));
    let launch = JobLaunch {
        job: job::Job {
            id: id.clone(),
            origin: job::Origin {
                soul: "soul_probe".to_string(),
                strand: "strand_probe".to_string(),
                turn: "turn_probe".to_string(),
                call: "call_probe".to_string(),
                effect: "effect_probe".to_string(),
            },
            description: "systemd production adapter probe".to_string(),
            command: r#"printf 'capability=%s\nsoul=%s\n' "${SANTI_JOB_CREATE_CAPABILITY-unset}" "$SANTI_SOUL_ID"; printf 'stderr-probe\n' >&2; exit 7"#.to_string(),
            cwd: None,
            timeout_seconds: 30,
            output_limit_bytes: 4096,
            state: job::State::Accepted,
            reason: None,
            exit_code: None,
            created: "2026-07-27T00:00:00.000Z".to_string(),
            updated: "2026-07-27T00:00:00.000Z".to_string(),
            accepted: Some("2026-07-27T00:00:00.000Z".to_string()),
            started: None,
            finished: None,
            acknowledged: None,
        },
        generation: format!("generation_{}", uuid::Uuid::new_v4().simple()),
        supervisor: supervisor_ref.clone(),
        cwd: temp.path().display().to_string(),
        directory: temp.path().join("job").display().to_string(),
    };
    let guard = Guard {
        supervisor: &supervisor,
        launch: &launch,
    };

    supervisor.ensure(&launch).expect("ensure transient job");
    let terminal = terminal(&supervisor, &launch);
    assert_eq!(terminal.state, job::State::Failed);
    assert_eq!(terminal.exit, Some(7));

    let stdout = std::fs::read_to_string(path(&launch).join("stdout.log")).expect("stdout");
    let stderr = std::fs::read_to_string(path(&launch).join("stderr.log")).expect("stderr");
    assert!(stdout.contains("capability=unset"), "{stdout}");
    assert!(stdout.contains("soul=soul_probe"), "{stdout}");
    assert_eq!(stderr, "stderr-probe\n");

    supervisor
        .acknowledge(&launch)
        .expect("acknowledge transient unit");
    assert_eq!(state(&supervisor_ref), "not-found");
    std::mem::forget(guard);
}

#[test]
fn bounds() {
    if !available() {
        eprintln!("skipping systemd job test: user manager is unavailable");
        return;
    }
    let temp = tempfile::tempdir().expect("temp dir");
    let (supervisor, launch) = launch(
        &temp,
        "output limit probe",
        "while true; do printf '0123456789abcdef'; done",
        1024,
    );
    let guard = Guard {
        supervisor: &supervisor,
        launch: &launch,
    };

    supervisor.ensure(&launch).expect("ensure transient job");
    let terminal = terminal(&supervisor, &launch);
    assert_eq!(terminal.state, job::State::Failed);
    assert_eq!(terminal.reason.as_deref(), Some("output_limit"));
    let length = std::fs::metadata(path(&launch).join("stdout.log"))
        .expect("stdout metadata")
        .len();
    assert_eq!(length, 1024);

    supervisor.acknowledge(&launch).expect("acknowledge job");
    assert_eq!(state(&launch.supervisor), "not-found");
    std::mem::forget(guard);
}

#[test]
fn times() {
    if !available() {
        eprintln!("skipping systemd job test: user manager is unavailable");
        return;
    }
    let temp = tempfile::tempdir().expect("temp dir");
    let (supervisor, mut launch) = launch(&temp, "runtime limit probe", "sleep 30", 4096);
    launch.job.timeout_seconds = 1;
    let guard = Guard {
        supervisor: &supervisor,
        launch: &launch,
    };

    supervisor.ensure(&launch).expect("ensure transient job");
    let terminal = terminal(&supervisor, &launch);
    assert_eq!(terminal.state, job::State::TimedOut);
    assert_eq!(terminal.reason.as_deref(), Some("runtime_limit"));

    supervisor.acknowledge(&launch).expect("acknowledge job");
    assert_eq!(state(&launch.supervisor), "not-found");
    std::mem::forget(guard);
}

#[test]
fn cancels() {
    if !available() {
        eprintln!("skipping systemd job test: user manager is unavailable");
        return;
    }
    let temp = tempfile::tempdir().expect("temp dir");
    let command = r#"printf '%s\n' $$ > main.pid; bash -c 'printf "%s\n" $$ > child.pid; sleep 300 & printf "%s\n" $! > grandchild.pid; wait' & wait"#;
    let (supervisor, launch) = launch(&temp, "process tree probe", command, 4096);
    let guard = Guard {
        supervisor: &supervisor,
        launch: &launch,
    };
    supervisor.ensure(&launch).expect("ensure transient job");
    let cwd = std::path::PathBuf::from(&launch.cwd);
    for _ in 0..100 {
        if cwd.join("grandchild.pid").is_file() {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    let pids = ["main.pid", "child.pid", "grandchild.pid"].map(|name| {
        std::fs::read_to_string(cwd.join(name))
            .expect("pid file")
            .trim()
            .to_string()
    });

    supervisor.stop(&launch).expect("stop process tree");
    let terminal = terminal(&supervisor, &launch);
    assert_eq!(terminal.state, job::State::Cancelled);
    for pid in pids {
        assert!(!alive(&pid), "pid {pid} survived job cancellation");
    }

    supervisor.acknowledge(&launch).expect("acknowledge job");
    assert_eq!(state(&launch.supervisor), "not-found");
    std::mem::forget(guard);
}
