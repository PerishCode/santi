use std::process::Command;

#[test]
fn storage_doctor_is_current() {
    let temp = tempfile::tempdir().expect("temp dir");
    let runtime = temp.path().join("runtime");
    let database = runtime.join("db");
    santi_core::Store::open(&database).expect("current-schema store");

    let output = Command::new(env!("CARGO_BIN_EXE_santi"))
        .args(["doctor", "--storage-only"])
        .current_dir(temp.path())
        .env("SANTI_PATHS_DATABASE", &database)
        .env("SANTI_PATHS_RUNTIME_ROOT", &runtime)
        .output()
        .expect("run storage doctor");

    assert!(
        output.status.success(),
        "storage doctor failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("doctor JSON");
    assert_eq!(report["schema_version"], santi_core::VERSION);
    assert_eq!(report["expected_schema_version"], santi_core::VERSION);
    assert_eq!(report["provider"], serde_json::Value::Null);
    assert_eq!(report["ok"], true);
}
