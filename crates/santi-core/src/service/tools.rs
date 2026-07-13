use std::{
    io::Read,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use santi_provider::ProviderFunctionCall;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    EffectState, EffectTransitionReason, SantiStreamPayload, WorkspaceRoot, parse_workspace_uri,
};

use super::SantiService;

impl SantiService {
    pub(super) fn handle_tool_call(
        &self,
        strand_id: &str,
        turn_id: &str,
        call: ProviderFunctionCall,
        output_limit: Option<usize>,
    ) -> Result<(), String> {
        // Persist the provider's raw item + ids so the Responses adapter can
        // replay the call verbatim; chat_completions rebuilds from name/args.
        let effect_type = (call.name == "shell").then_some("shell");
        let (tool_call, effect) = self.store.append_effect_call(
            turn_id,
            &call.call_id,
            &call.name,
            &call.arguments,
            &crate::ToolCallProvenance {
                provider_family: self.provider.metadata().provider.to_string(),
                item: Some(call.item.clone()),
                item_id: call.item_id.clone(),
                response_id: Some(call.response_id.clone()),
            },
            effect_type,
        )?;
        self.publish_stream(
            strand_id,
            SantiStreamPayload::ToolCallCreated {
                tool_call: tool_call.clone(),
            },
        );
        let result = if let Some(effect) = effect {
            self.handle_shell_effect(strand_id, turn_id, &call, &effect.id, output_limit)?
        } else {
            self.store.append_tool_result(
                &call.call_id,
                None,
                Some(bounded_tool_error(
                    format!("unsupported tool: {}", call.name),
                    output_limit,
                )),
            )?
        };
        self.publish_stream(
            strand_id,
            SantiStreamPayload::ToolResultCreated {
                tool_result: result,
            },
        );
        Ok(())
    }

    fn handle_shell_effect(
        &self,
        strand_id: &str,
        turn_id: &str,
        call: &ProviderFunctionCall,
        effect_id: &str,
        output_limit: Option<usize>,
    ) -> Result<crate::ToolResult, String> {
        let soul_id = self.store.soul_id_for_strand(strand_id)?;
        let prepared = match parse_tool_args::<ShellArgs>(&call.arguments)
            .and_then(|args| self.prepare_shell(strand_id, turn_id, &soul_id, args))
        {
            Ok(prepared) => prepared,
            Err(error) => {
                return self.store.append_effect_tool_result(
                    effect_id,
                    &call.call_id,
                    None,
                    Some(bounded_tool_error(error, output_limit)),
                    EffectState::NotDispatched,
                );
            }
        };
        self.store.begin_effect_dispatch(effect_id)?;
        match run_prepared_shell(prepared, output_limit) {
            ShellDispatchOutcome::Captured(output) => self.store.append_effect_tool_result(
                effect_id,
                &call.call_id,
                Some(output),
                None,
                EffectState::Confirmed,
            ),
            ShellDispatchOutcome::NotDispatched(error) => self.store.append_effect_tool_result(
                effect_id,
                &call.call_id,
                None,
                Some(bounded_tool_error(error, output_limit)),
                EffectState::NotDispatched,
            ),
            ShellDispatchOutcome::Unknown(error) => {
                self.store.mark_effect_unknown(
                    effect_id,
                    EffectTransitionReason::ResultCaptureFailed,
                    &error,
                )?;
                Err(format!(
                    "shell effect {effect_id} outcome is unknown; automatic replay is forbidden: {error}"
                ))
            }
        }
    }

    fn prepare_shell(
        &self,
        strand_id: &str,
        turn_id: &str,
        soul_id: &str,
        args: ShellArgs,
    ) -> Result<PreparedShell, String> {
        std::fs::create_dir_all(self.soul_memory_dir(soul_id))
            .map_err(|error| error.to_string())?;
        std::fs::create_dir_all(self.strand_memory_dir(strand_id))
            .map_err(|error| error.to_string())?;
        let cwd = self.resolve_shell_cwd(strand_id, soul_id, args.cwd.as_deref())?;
        std::fs::create_dir_all(&cwd).map_err(|error| error.to_string())?;
        let mut command = shell_command(&args.command);
        command
            .current_dir(&cwd)
            .env("SANTI_SOUL_MEMORY_DIR", self.soul_memory_dir(soul_id))
            .env("SANTI_STRAND_MEMORY_DIR", self.strand_memory_dir(strand_id))
            // Self-involved: the soul inherits its own domain, so `santi …` from
            // its shell auto-scopes to itself + this strand (via the CLI's
            // --soul/--strand env defaults). Ambient capability, not authorization.
            .env("SANTI_SOUL_ID", soul_id)
            .env("SANTI_STRAND_ID", strand_id)
            .env("SANTI_TURN_ID", turn_id)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        Ok(PreparedShell { command, cwd })
    }

    fn resolve_shell_cwd(
        &self,
        strand_id: &str,
        soul_id: &str,
        cwd: Option<&str>,
    ) -> Result<PathBuf, String> {
        let Some(cwd) = cwd else {
            return Ok(self.execution_root());
        };
        let uri = parse_workspace_uri(cwd)?;
        let root = match uri.root {
            WorkspaceRoot::Soul => self.soul_memory_dir(soul_id),
            WorkspaceRoot::Strand => self.strand_memory_dir(strand_id),
        };
        Ok(root.join(uri.path))
    }

    pub(super) fn runtime_root(&self) -> PathBuf {
        PathBuf::from(&self.config.runtime_root)
    }

    pub(super) fn execution_root(&self) -> PathBuf {
        PathBuf::from(&self.config.execution_root)
    }

    pub(super) fn soul_memory_dir(&self, soul_id: &str) -> PathBuf {
        self.runtime_root()
            .join("souls")
            .join(soul_id)
            .join("memory")
    }

    pub(super) fn soul_memory_file(&self, soul_id: &str) -> PathBuf {
        // Delegate to the free function so offline ops (`santi doctor`) and the
        // running service always resolve the same path.
        crate::store::soul_memory_file(self.runtime_root(), soul_id)
    }

    pub(super) fn strand_memory_dir(&self, strand_id: &str) -> PathBuf {
        self.runtime_root()
            .join("strands")
            .join(strand_id)
            .join("memory")
    }

    pub(super) fn strand_memory_file(&self, strand_id: &str) -> PathBuf {
        self.strand_memory_dir(strand_id).join("MEMORY.md")
    }

    /// The `[santi]` constitution config file: `SANTI_CONSTITUTION_FILE` if set,
    /// else `<runtime_root>/constitution.md`. Absent → the encoded default. It
    /// is read per-turn (hot), so editing it takes effect on the next turn with
    /// no restart — the observe→refine loop.
    pub(super) fn constitution_file(&self) -> PathBuf {
        std::env::var("SANTI_CONSTITUTION_FILE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| self.runtime_root().join("constitution.md"))
    }
}

#[derive(Debug, Deserialize)]
struct ShellArgs {
    command: String,
    cwd: Option<String>,
}

struct PreparedShell {
    command: Command,
    cwd: PathBuf,
}

enum ShellDispatchOutcome {
    Captured(Value),
    NotDispatched(String),
    Unknown(String),
}

fn run_prepared_shell(
    prepared: PreparedShell,
    output_limit: Option<usize>,
) -> ShellDispatchOutcome {
    let PreparedShell { mut command, cwd } = prepared;
    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return ShellDispatchOutcome::NotDispatched(format!(
                "failed to spawn shell process: {error}"
            ));
        }
    };
    match output_limit {
        None => match child.wait_with_output() {
            Ok(output) => ShellDispatchOutcome::Captured(json!({
                "exit_code": output.status.code().unwrap_or(-1),
                "stdout": String::from_utf8_lossy(&output.stdout),
                "stderr": String::from_utf8_lossy(&output.stderr),
                "shell": default_shell_name(),
                "cwd": cwd.display().to_string(),
            })),
            Err(error) => ShellDispatchOutcome::Unknown(format!(
                "shell process was spawned but its result could not be captured: {error}"
            )),
        },
        Some(limit) => wait_with_bounded_output(child, cwd, limit),
    }
}

struct CapturedPipe {
    bytes: Vec<u8>,
    truncated: bool,
}

fn wait_with_bounded_output(mut child: Child, cwd: PathBuf, limit: usize) -> ShellDispatchOutcome {
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return ShellDispatchOutcome::Unknown(
            "shell stdout or stderr pipe was unavailable".to_string(),
        );
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
            return ShellDispatchOutcome::Unknown(format!(
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
    ShellDispatchOutcome::Captured(json!({
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
) -> std::thread::JoinHandle<Result<CapturedPipe, String>>
where
    R: Read + Send + 'static,
{
    std::thread::spawn(move || capture_pipe(reader, &remaining))
}

fn capture_pipe(mut reader: impl Read, remaining: &AtomicUsize) -> Result<CapturedPipe, String> {
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
    Ok(CapturedPipe { bytes, truncated })
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
    handle: std::thread::JoinHandle<Result<CapturedPipe, String>>,
    name: &str,
) -> Result<CapturedPipe, String> {
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

fn bounded_tool_error(error: String, limit: Option<usize>) -> String {
    let Some(limit) = limit else {
        return error;
    };
    if error.len() <= limit {
        return error;
    }
    let mut end = limit;
    while end > 0 && !error.is_char_boundary(end) {
        end -= 1;
    }
    error[..end].to_string()
}

fn shell_command(command: &str) -> Command {
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

fn parse_tool_args<T: for<'de> Deserialize<'de>>(value: &Value) -> Result<T, String> {
    serde_json::from_value(value.clone()).map_err(|error| error.to_string())
}
