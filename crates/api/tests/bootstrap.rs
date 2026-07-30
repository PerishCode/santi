use std::process::{Command, Output};

fn bootstrap(config: &std::path::Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_santi-api"))
        .args(["--config", &config.display().to_string(), "bootstrap"])
        .output()
        .expect("run bootstrap")
}

#[test]
fn custody() {
    let temp = tempfile::tempdir().expect("temp");
    let runtime = temp.path().join("runtime");
    let database = runtime.join("db");
    let config = temp.path().join("santi.toml");
    std::fs::write(
        &config,
        format!(
            "[paths]\ndatabase = {:?}\nruntime = {:?}\n",
            database.display().to_string(),
            runtime.display().to_string(),
        ),
    )
    .expect("write config");

    let first = bootstrap(&config);
    assert!(
        first.status.success(),
        "bootstrap failed: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&first.stdout).expect("bootstrap report");
    assert_eq!(report["custody_created"], true);

    let custody = runtime.join("sudo");
    let sudo = std::fs::read_to_string(&custody).expect("read custody");
    assert_eq!(sudo.len(), 64);
    assert!(
        sudo.bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&custody)
                .expect("custody metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    let second = bootstrap(&config);
    assert!(second.status.success());
    let report: serde_json::Value = serde_json::from_slice(&second.stdout).expect("replay report");
    assert_eq!(report["custody_created"], false);
    assert_eq!(
        std::fs::read_to_string(&custody).expect("read replay custody"),
        sudo
    );

    std::fs::remove_file(&custody).expect("simulate lost custody");
    let lost = bootstrap(&config);
    assert!(!lost.status.success());
    assert!(
        String::from_utf8_lossy(&lost.stderr)
            .contains("sudo custody is absent for an occupied estate")
    );
}
