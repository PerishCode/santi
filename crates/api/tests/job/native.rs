#[cfg(target_os = "windows")]
use std::path::PathBuf;
use std::process::Command;

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn available() -> bool {
    #[cfg(target_os = "linux")]
    let mut command = {
        let mut command = Command::new("systemctl");
        command.args(["--user", "show-environment"]);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("launchctl");
        command.args(["print", &domain()]);
        command
    };
    command
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(target_os = "windows")]
pub fn available() -> bool {
    true
}

#[cfg(unix)]
pub fn alive(pid: &str) -> bool {
    Command::new("kill")
        .args(["-0", pid])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(target_os = "windows")]
pub fn alive(pid: &str) -> bool {
    let Some(system) = std::env::var_os("SYSTEMROOT").map(PathBuf::from) else {
        return false;
    };
    let output = Command::new(system.join("System32").join("tasklist.exe"))
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output();
    output.is_ok_and(|output| {
        output.status.success()
            && String::from_utf8_lossy(&output.stdout).contains(&format!(",\"{pid}\","))
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn state(unit: &str) -> String {
    #[cfg(target_os = "linux")]
    {
        let output = Command::new("systemctl")
            .args(["--user", "show", unit, "--property=LoadState", "--value"])
            .output()
            .expect("inspect load state");
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }
    #[cfg(target_os = "macos")]
    {
        if Command::new("launchctl")
            .args(["print", &target(unit)])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
        {
            "loaded".to_string()
        } else {
            "not-found".to_string()
        }
    }
}

#[cfg(target_os = "windows")]
pub fn state(_unit: &str) -> String {
    "not-found".to_string()
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
