use std::{
    io::Read,
    path::PathBuf,
    process::{Child, Command},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
pub(super) struct Args {
    pub(super) command: String,
    pub(super) cwd: Option<String>,
}

pub(super) struct Prepared {
    pub(super) command: Command,
    pub(super) cwd: PathBuf,
}

pub(super) enum Outcome {
    Captured(Value),
    Failed(String),
    Unknown(String),
}

pub(super) fn ran(prepared: Prepared, output_limit: Option<usize>) -> Outcome {
    let Prepared { mut command, cwd } = prepared;
    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return Outcome::Failed(format!("failed to spawn shell process: {error}"));
        }
    };
    match output_limit {
        None => match child.wait_with_output() {
            Ok(output) => Outcome::Captured(json!({
                "exit_code": output.status.code().unwrap_or(-1),
                "stdout": String::from_utf8_lossy(&output.stdout),
                "stderr": String::from_utf8_lossy(&output.stderr),
                "shell": sheller(),
                "cwd": cwd.display().to_string(),
            })),
            Err(error) => Outcome::Unknown(format!(
                "shell process was spawned but its result could not be captured: {error}"
            )),
        },
        Some(limit) => capped(child, cwd, limit),
    }
}

struct Pipe {
    bytes: Vec<u8>,
    truncated: bool,
}

fn capped(mut child: Child, cwd: PathBuf, limit: usize) -> Outcome {
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Outcome::Unknown("shell stdout or stderr pipe was unavailable".to_string());
    };
    let remaining = Arc::new(AtomicUsize::new(limit));
    let stdout = spawned(stdout, remaining.clone());
    let stderr = spawned(stderr, remaining);
    let status = child.wait().inspect_err(|_| {
        let _ = child.kill();
        let _ = child.wait();
    });
    let stdout = joined(stdout, "stdout");
    let stderr = joined(stderr, "stderr");
    let (status, stdout, stderr) = match (status, stdout, stderr) {
        (Ok(status), Ok(stdout), Ok(stderr)) => (status, stdout, stderr),
        (status, stdout, stderr) => {
            return Outcome::Unknown(format!(
                "shell process was spawned but its bounded result could not be captured: status={}; stdout={}; stderr={}",
                shown(status),
                shown(stdout),
                shown(stderr),
            ));
        }
    };
    let (stdout_text, stdout_text_truncated) = lossy(&stdout.bytes, limit);
    let remaining = limit.saturating_sub(stdout_text.len());
    let (stderr_text, stderr_text_truncated) = lossy(&stderr.bytes, remaining);
    let truncated =
        stdout.truncated || stderr.truncated || stdout_text_truncated || stderr_text_truncated;
    Outcome::Captured(json!({
        "exit_code": status.code().unwrap_or(-1),
        "stdout": stdout_text,
        "stderr": stderr_text,
        "shell": sheller(),
        "cwd": cwd.display().to_string(),
        "output_truncated": truncated,
        "output_limit_bytes": limit,
    }))
}

fn spawned<R>(
    reader: R,
    remaining: Arc<AtomicUsize>,
) -> std::thread::JoinHandle<Result<Pipe, String>>
where
    R: Read + Send + 'static,
{
    std::thread::spawn(move || piped(reader, &remaining))
}

fn piped(mut reader: impl Read, remaining: &AtomicUsize) -> Result<Pipe, String> {
    let mut bytes = Vec::new();
    let mut truncated = false;
    let mut chunk = [0u8; 8192];
    loop {
        let read = reader.read(&mut chunk).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        let keep = reserved(remaining, read);
        bytes.extend_from_slice(&chunk[..keep]);
        truncated |= keep < read;
    }
    Ok(Pipe { bytes, truncated })
}

fn reserved(remaining: &AtomicUsize, requested: usize) -> usize {
    let mut available = remaining.load(Ordering::Acquire);
    loop {
        let reserved = available.min(requested);
        match remaining.compare_exchange_weak(
            available,
            available - reserved,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return reserved,
            Err(actual) => available = actual,
        }
    }
}

fn joined(
    handle: std::thread::JoinHandle<Result<Pipe, String>>,
    name: &str,
) -> Result<Pipe, String> {
    handle
        .join()
        .map_err(|_| format!("{name} capture thread panicked"))?
}

fn shown<T, E: std::fmt::Display>(result: Result<T, E>) -> String {
    match result {
        Ok(_) => "ok".to_string(),
        Err(error) => error.to_string(),
    }
}

fn lossy(bytes: &[u8], limit: usize) -> (String, bool) {
    let text = String::from_utf8_lossy(bytes);
    if text.len() <= limit {
        return (text.into_owned(), false);
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_string(), true)
}
pub(super) fn shell(command: &str) -> Command {
    #[cfg(windows)]
    {
        let mut shell = Command::new("pwsh");
        shell
            .arg("-NoLogo")
            .arg("-NoProfile")
            .arg("-Command")
            .arg(command);
        shell
    }

    #[cfg(not(windows))]
    {
        let mut shell = Command::new("/bin/bash");
        shell.arg("-lc").arg(command);
        shell
    }
}

fn sheller() -> &'static str {
    if cfg!(windows) { "pwsh" } else { "bash" }
}
