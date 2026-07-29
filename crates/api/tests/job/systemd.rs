use std::{thread, time::Duration};

use santi_core::{
    job,
    service::{JobLaunch, JobSupervisor},
};

use super::support::{Guard, alive, available, launch, path, state, terminal};

#[test]
fn retains() {
    if !available() {
        eprintln!("skipping native job test: user supervisor is unavailable");
        return;
    }
    let temp = tempfile::tempdir().expect("temp dir");
    let id = format!("job_{}", uuid::Uuid::new_v4().simple());
    let stamp = format!("stamp_{}", uuid::Uuid::new_v4().simple());
    let sidecar = format!("santi-{}.service", stamp.replace('_', "-"));
    let supervisor = santi_api::jobs::Native::new(env!("CARGO_BIN_EXE_santi-api"));
    #[cfg(unix)]
    let command = r#"printf 'capability=%s\nsoul=%s\n' "${SANTI_JOB_CREATE_CAPABILITY-unset}" "$SANTI_SOUL_ID"; printf 'stderr-probe\n' >&2; exit 7"#;
    #[cfg(target_os = "windows")]
    let command = r#"Write-Output "capability=$env:SANTI_JOB_CREATE_CAPABILITY"; Write-Output "soul=$env:SANTI_SOUL_ID"; [Console]::Error.WriteLine("stderr-probe"); exit 7"#;
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
            description: "native production adapter probe".to_string(),
            command: command.to_string(),
            cwd: None,
            timeout_seconds: 30,
            output_limit_bytes: 4096,
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
        sidecar: sidecar.clone(),
        cwd: temp.path().display().to_string(),
        directory: temp.path().join("job").display().to_string(),
    };
    let guard = Guard {
        supervisor: &supervisor,
        launch: &launch,
    };

    supervisor.detach(&launch).expect("detach transient job");
    let terminal = terminal(&supervisor, &launch);
    assert_eq!(terminal.state, job::State::Failed);
    assert_eq!(terminal.exit, Some(7));

    let stdout = std::fs::read_to_string(path(&launch).join("stdout.log")).expect("stdout");
    let stderr = std::fs::read_to_string(path(&launch).join("stderr.log")).expect("stderr");
    #[cfg(unix)]
    assert!(stdout.contains("capability=unset"), "{stdout}");
    #[cfg(target_os = "windows")]
    assert!(stdout.contains("capability="), "{stdout}");
    assert!(stdout.contains("soul=soul_probe"), "{stdout}");
    assert_eq!(stderr, "stderr-probe\n");

    supervisor
        .acknowledge(&launch)
        .expect("acknowledge transient unit");
    assert_eq!(state(&sidecar), "not-found");
    std::mem::forget(guard);
}

#[test]
fn bounds() {
    if !available() {
        eprintln!("skipping native job test: user supervisor is unavailable");
        return;
    }
    let temp = tempfile::tempdir().expect("temp dir");
    #[cfg(unix)]
    let command = "while true; do printf '0123456789abcdef'; done";
    #[cfg(target_os = "windows")]
    let command = r#"while($true){[Console]::Out.Write("0123456789abcdef")}"#;
    let (supervisor, launch) = launch(&temp, "output limit probe", command, 1024);
    let guard = Guard {
        supervisor: &supervisor,
        launch: &launch,
    };

    supervisor.detach(&launch).expect("detach transient job");
    let terminal = terminal(&supervisor, &launch);
    assert_eq!(terminal.state, job::State::Failed);
    assert_eq!(terminal.reason.as_deref(), Some("output_limit"));
    let length = std::fs::metadata(path(&launch).join("stdout.log"))
        .expect("stdout metadata")
        .len();
    assert_eq!(length, 1024);

    supervisor.acknowledge(&launch).expect("acknowledge job");
    assert_eq!(state(&launch.sidecar), "not-found");
    std::mem::forget(guard);
}

#[test]
fn times() {
    if !available() {
        eprintln!("skipping native job test: user supervisor is unavailable");
        return;
    }
    let temp = tempfile::tempdir().expect("temp dir");
    #[cfg(unix)]
    let command = "sleep 30";
    #[cfg(target_os = "windows")]
    let command = "Start-Sleep -Seconds 30";
    let (supervisor, mut launch) = launch(&temp, "runtime limit probe", command, 4096);
    launch.job.timeout_seconds = 1;
    let guard = Guard {
        supervisor: &supervisor,
        launch: &launch,
    };

    supervisor.detach(&launch).expect("detach transient job");
    let terminal = terminal(&supervisor, &launch);
    assert_eq!(terminal.state, job::State::TimedOut);
    assert_eq!(terminal.reason.as_deref(), Some("runtime_limit"));

    supervisor.acknowledge(&launch).expect("acknowledge job");
    assert_eq!(state(&launch.sidecar), "not-found");
    std::mem::forget(guard);
}

#[test]
fn cancels() {
    if !available() {
        eprintln!("skipping native job test: user supervisor is unavailable");
        return;
    }
    let temp = tempfile::tempdir().expect("temp dir");
    #[cfg(unix)]
    let command = r#"printf '%s\n' $$ > main.pid; bash -c 'printf "%s\n" $$ > child.pid; sleep 300 & printf "%s\n" $! > grandchild.pid; wait' & wait"#;
    #[cfg(target_os = "windows")]
    let command = r#"Set-Content -NoNewline main.pid $PID; $child=Start-Process powershell.exe -ArgumentList "-NoLogo","-NoProfile","-NonInteractive","-Command","Start-Sleep -Seconds 300" -PassThru; Set-Content -NoNewline child.pid $child.Id; Wait-Process -Id $child.Id"#;
    let (supervisor, launch) = launch(&temp, "process tree probe", command, 4096);
    let guard = Guard {
        supervisor: &supervisor,
        launch: &launch,
    };
    supervisor.detach(&launch).expect("detach transient job");
    let cwd = std::path::PathBuf::from(&launch.cwd);
    #[cfg(unix)]
    let names = ["main.pid", "child.pid", "grandchild.pid"].as_slice();
    #[cfg(target_os = "windows")]
    let names = ["main.pid", "child.pid"].as_slice();
    for _ in 0..100 {
        if names.iter().all(|name| cwd.join(name).is_file()) {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    let pids = names
        .iter()
        .map(|name| {
            std::fs::read_to_string(cwd.join(name))
                .expect("pid file")
                .trim()
                .to_string()
        })
        .collect::<Vec<_>>();

    supervisor.stop(&launch).expect("stop process tree");
    let terminal = terminal(&supervisor, &launch);
    assert_eq!(terminal.state, job::State::Cancelled);
    for pid in pids {
        assert!(!alive(&pid), "pid {pid} survived job cancellation");
    }

    supervisor.acknowledge(&launch).expect("acknowledge job");
    assert_eq!(state(&launch.sidecar), "not-found");
    std::mem::forget(guard);
}
