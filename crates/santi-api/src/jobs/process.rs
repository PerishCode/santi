use std::{
    fs::File,
    io::{Read, Write},
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use santi_core::job;

mod command;
mod control;

use control::Halt;

use super::{
    DIRECTORY, GENERATION, STAMP, files,
    model::{Phase, Record, Spec},
};

pub(super) fn run() -> Result<(), String> {
    let directory = directory()?;
    let spec = files::directory(&directory).spec()?;
    verify(&spec)?;
    let outlog = files::directory(&directory).log("stdout.log")?;
    let errlog = files::directory(&directory).log("stderr.log")?;
    let remaining = Arc::new(AtomicU64::new(spec.output));
    let exceeded = Arc::new(AtomicBool::new(false));
    #[cfg(target_os = "windows")]
    let _lease = files::directory(&directory).lease()?;
    files::directory(&directory).advance(Phase::Claimed)?;

    let mut command = command::build(&spec);
    command
        .current_dir(&spec.cwd)
        .env_clear()
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    santi_core::environment::allow(&mut command);
    command
        .env("SANTI_SOUL_ID", &spec.origin.soul)
        .env("SANTI_STRAND_ID", &spec.origin.strand)
        .env("SANTI_TURN_ID", &spec.origin.turn)
        .env("SANTI_TOOL_CALL_ID", &spec.origin.call)
        .env("SANTI_EFFECT_ID", &spec.origin.effect)
        .env("SANTI_JOB_ID", &spec.id);
    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to spawn job command: {error}"))?;
    if let Err(error) = files::directory(&directory).advance(Phase::Running) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "job stdout pipe is unavailable".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "job stderr pipe is unavailable".to_string())?;
    let out = copy(Sink {
        reader: stdout,
        output: outlog,
        remaining: remaining.clone(),
        exceeded: exceeded.clone(),
        directory: directory.clone(),
    });
    let err = copy(Sink {
        reader: stderr,
        output: errlog,
        remaining,
        exceeded: exceeded.clone(),
        directory: directory.clone(),
    });
    let (status, halt) = control::wait(&mut child, &directory, &exceeded)?;
    join(out)?;
    join(err)?;
    let terminal = match halt {
        Some(Halt::Output) => Record {
            state: job::State::Failed,
            reason: Some("output_limit".to_string()),
            exit: status.code(),
        },
        Some(Halt::Cancel) => Record {
            state: job::State::Cancelled,
            reason: Some("cancel_requested".to_string()),
            exit: status.code(),
        },
        Some(Halt::Timeout) => Record {
            state: job::State::TimedOut,
            reason: Some("runtime_limit".to_string()),
            exit: status.code(),
        },
        None if status.success() => Record {
            state: job::State::Succeeded,
            reason: None,
            exit: Some(0),
        },
        None => Record {
            state: job::State::Failed,
            reason: Some("exit_code".to_string()),
            exit: status.code(),
        },
    };
    finish(&directory, &terminal)?;
    if terminal.state == job::State::Succeeded {
        Ok(())
    } else {
        std::process::exit(status.code().unwrap_or(1));
    }
}

pub(super) fn finalize() -> Result<(), String> {
    let directory = directory()?;
    if files::directory(&directory).terminal()?.is_some() {
        return Ok(());
    }
    let code = std::env::var("EXIT_CODE").unwrap_or_else(|_| "unknown".to_string());
    let raw = std::env::var("EXIT_STATUS").unwrap_or_default();
    let status = (code == "exited")
        .then(|| raw.parse::<i32>().ok())
        .flatten();
    let terminal = if directory.join(files::LIMIT).is_file() {
        Record {
            state: job::State::Failed,
            reason: Some("output_limit".to_string()),
            exit: status,
        }
    } else if directory.join(files::CANCEL).is_file() {
        Record {
            state: job::State::Cancelled,
            reason: Some("cancel_requested".to_string()),
            exit: status,
        }
    } else {
        match std::env::var("SERVICE_RESULT").as_deref() {
            Ok("timeout") => Record {
                state: job::State::TimedOut,
                reason: Some("runtime_limit".to_string()),
                exit: status,
            },
            Ok("success") => Record {
                state: job::State::Succeeded,
                reason: None,
                exit: Some(0),
            },
            Ok(reason) => Record {
                state: job::State::Failed,
                reason: Some(reason.to_string()),
                exit: status,
            },
            Err(_) => Record {
                state: job::State::Failed,
                reason: Some("unknown".to_string()),
                exit: status,
            },
        }
    };
    finish(&directory, &terminal)
}

struct Sink<R> {
    reader: R,
    output: File,
    remaining: Arc<AtomicU64>,
    exceeded: Arc<AtomicBool>,
    directory: PathBuf,
}

fn copy<R: Read + Send + 'static>(
    mut sink: Sink<R>,
) -> std::thread::JoinHandle<Result<(), String>> {
    std::thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            let read = sink
                .reader
                .read(&mut buffer)
                .map_err(|error| error.to_string())?;
            if read == 0 {
                return sink.output.flush().map_err(|error| error.to_string());
            }
            let keep = reserve(&sink.remaining, read as u64) as usize;
            if keep > 0 {
                sink.output
                    .write_all(&buffer[..keep])
                    .map_err(|error| error.to_string())?;
                sink.output.flush().map_err(|error| error.to_string())?;
            }
            if keep < read && !sink.exceeded.swap(true, Ordering::AcqRel) {
                files::directory(&sink.directory).mark(files::LIMIT)?;
            }
        }
    })
}

fn join(handle: std::thread::JoinHandle<Result<(), String>>) -> Result<(), String> {
    handle
        .join()
        .map_err(|_| "job output thread panicked".to_string())?
}

fn reserve(remaining: &AtomicU64, requested: u64) -> u64 {
    let mut available = remaining.load(Ordering::Acquire);
    loop {
        let kept = available.min(requested);
        match remaining.compare_exchange_weak(
            available,
            available - kept,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return kept,
            Err(actual) => available = actual,
        }
    }
}

fn directory() -> Result<PathBuf, String> {
    std::env::var(DIRECTORY)
        .map(PathBuf::from)
        .map_err(|_| format!("{DIRECTORY} is missing"))
}

fn verify(spec: &Spec) -> Result<(), String> {
    let stamp = std::env::var(STAMP)
        .or_else(|_| std::env::var(GENERATION))
        .map_err(|_| format!("{STAMP} is missing"))?;
    if stamp == spec.stamp {
        Ok(())
    } else {
        Err("job stamp does not match its accepted spec".to_string())
    }
}

fn finish(directory: &std::path::Path, terminal: &Record) -> Result<(), String> {
    files::directory(directory).finish(terminal)?;
    if files::directory(directory).state()?.is_some() {
        files::directory(directory).advance(Phase::Terminal)?;
    }
    Ok(())
}
