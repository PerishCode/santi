use super::Spec;
use std::process::Command;

#[cfg(unix)]
pub(super) fn build(spec: &Spec) -> Command {
    use std::os::unix::process::CommandExt;

    let mut command = Command::new("/bin/bash");
    command.args(["-lc", &spec.command]).process_group(0);
    command
}

#[cfg(target_os = "windows")]
pub(super) fn build(spec: &Spec) -> Command {
    use std::os::windows::process::CommandExt;

    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &spec.command,
        ])
        .creation_flags(CREATE_NEW_PROCESS_GROUP);
    command
}
