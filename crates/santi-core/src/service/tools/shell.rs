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

pub(super) fn run_prepared_shell(prepared: Prepared, output_limit: Option<usize>) -> Outcome {
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
                "shell": default_shell_name(),
                "cwd": cwd.display().to_string(),
            })),
            Err(error) => Outcome::Unknown(format!(
                "shell process was spawned but its result could not be captured: {error}"
            )),
        },
        Some(limit) => wait_with_bounded_output(child, cwd, limit),
    }
}

struct Pipe {
    bytes: Vec<u8>,
    truncated: bool,
}

fn wait_with_bounded_output(mut child: Child, cwd: PathBuf, limit: usize) -> Outcome {
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Outcome::Unknown("shell stdout or stderr pipe was unavailable".to_string());
    };
    let remaining = Arc::new(AtomicUsize::new(limit));
    let stdout_capture = spawn_pipe_capture(stdout, remaining.clone());
    let stderr_capture = spawn_pipe_capture(stderr, remaining);
    let status = child.wait().inspect_err(|_| {
        let _ = child.kill();
        let _ = child.wait();
    });
    let stdout = join_pipe_capture(stdout_capture, "stdout");
    let stderr = join_pipe_capture(stderr_capture, "stderr");
    let (status, stdout, stderr) = match (status, stdout, stderr) {
        (Ok(status), Ok(stdout), Ok(stderr)) => (status, stdout, stderr),
        (status, stdout, stderr) => {
            return Outcome::Unknown(format!(
                "shell process was spawned but its bounded result could not be captured: status={}; stdout={}; stderr={}",
                capture_status(status),
                capture_status(stdout),
                capture_status(stderr),
            ));
        }
    };
    let (stdout_text, stdout_text_truncated) = lossy_prefix(&stdout.bytes, limit);
    let text_remaining = limit.saturating_sub(stdout_text.len());
    let (stderr_text, stderr_text_truncated) = lossy_prefix(&stderr.bytes, text_remaining);
    let output_truncated =
        stdout.truncated || stderr.truncated || stdout_text_truncated || stderr_text_truncated;
    Outcome::Captured(json!({
        "exit_code": status.code().unwrap_or(-1),
        "stdout": stdout_text,
        "stderr": stderr_text,
        "shell": default_shell_name(),
        "cwd": cwd.display().to_string(),
        "output_truncated": output_truncated,
        "output_limit_bytes": limit,
    }))
}

fn spawn_pipe_capture<R>(
    reader: R,
    remaining: Arc<AtomicUsize>,
) -> std::thread::JoinHandle<Result<Pipe, String>>
where
    R: Read + Send + 'static,
{
    std::thread::spawn(move || capture_pipe(reader, &remaining))
}

fn capture_pipe(mut reader: impl Read, remaining: &AtomicUsize) -> Result<Pipe, String> {
    let mut bytes = Vec::new();
    let mut truncated = false;
    let mut chunk = [0u8; 8192];
    loop {
        let read = reader.read(&mut chunk).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        let keep = reserve_capture_bytes(remaining, read);
        bytes.extend_from_slice(&chunk[..keep]);
        truncated |= keep < read;
    }
    Ok(Pipe { bytes, truncated })
}

fn reserve_capture_bytes(remaining: &AtomicUsize, requested: usize) -> usize {
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

fn join_pipe_capture(
    handle: std::thread::JoinHandle<Result<Pipe, String>>,
    name: &str,
) -> Result<Pipe, String> {
    handle
        .join()
        .map_err(|_| format!("{name} capture thread panicked"))?
}

fn capture_status<T, E: std::fmt::Display>(result: Result<T, E>) -> String {
    match result {
        Ok(_) => "ok".to_string(),
        Err(error) => error.to_string(),
    }
}

fn lossy_prefix(bytes: &[u8], limit: usize) -> (String, bool) {
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
pub(super) fn shell_command(command: &str) -> Command {
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

fn default_shell_name() -> &'static str {
    if cfg!(windows) { "pwsh" } else { "bash" }
}
