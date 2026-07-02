use std::{path::PathBuf, process::Command};

use santi_provider::ProviderFunctionCall;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{SantiStreamPayload, WorkspaceRoot, parse_workspace_uri};

use super::SantiService;

impl SantiService {
    pub(super) fn handle_tool_call(
        &self,
        strand_id: &str,
        turn_id: &str,
        call: ProviderFunctionCall,
    ) -> Result<(), String> {
        // Persist the provider's raw item + ids so the Responses adapter can
        // replay the call verbatim; chat_completions rebuilds from name/args.
        let tool_call = self.store.append_tool_call(
            turn_id,
            &call.call_id,
            &call.name,
            &call.arguments,
            &crate::ToolCallProvenance {
                item: Some(call.item.clone()),
                item_id: call.item_id.clone(),
                response_id: Some(call.response_id.clone()),
            },
        )?;
        self.publish_stream(
            strand_id,
            SantiStreamPayload::ToolCallCreated {
                tool_call: tool_call.clone(),
            },
        );
        let soul_id = self.store.soul_id_for_strand(strand_id)?;
        let dispatch = self.dispatch_tool(strand_id, &soul_id, &call);
        let (output, error_text) = match dispatch {
            Ok(output) => (Some(output), None),
            Err(error) => (None, Some(error)),
        };
        let result = self
            .store
            .append_tool_result(&call.call_id, output, error_text)?;
        self.publish_stream(
            strand_id,
            SantiStreamPayload::ToolResultCreated {
                tool_result: result,
            },
        );
        Ok(())
    }

    fn dispatch_tool(
        &self,
        strand_id: &str,
        soul_id: &str,
        call: &ProviderFunctionCall,
    ) -> Result<Value, String> {
        match call.name.as_str() {
            "shell" => {
                let args = parse_tool_args::<ShellArgs>(&call.arguments)?;
                self.run_shell(strand_id, soul_id, args)
            }
            name => Err(format!("unsupported tool: {name}")),
        }
    }

    fn run_shell(&self, strand_id: &str, soul_id: &str, args: ShellArgs) -> Result<Value, String> {
        std::fs::create_dir_all(self.soul_memory_dir(soul_id))
            .map_err(|error| error.to_string())?;
        std::fs::create_dir_all(self.strand_memory_dir(strand_id))
            .map_err(|error| error.to_string())?;
        let cwd = self.resolve_shell_cwd(strand_id, soul_id, args.cwd.as_deref())?;
        std::fs::create_dir_all(&cwd).map_err(|error| error.to_string())?;
        let mut command = shell_command(&args.command);
        let output = command
            .current_dir(&cwd)
            .env("SANTI_SOUL_MEMORY_DIR", self.soul_memory_dir(soul_id))
            .env("SANTI_STRAND_MEMORY_DIR", self.strand_memory_dir(strand_id))
            // Self-involved: the soul inherits its own domain, so `santi …` from
            // its shell auto-scopes to itself + this strand (via the CLI's
            // --soul/--strand env defaults). Ambient capability, not authorization.
            .env("SANTI_SOUL_ID", soul_id)
            .env("SANTI_STRAND_ID", strand_id)
            .output()
            .map_err(|error| format!("failed to run shell: {error}"))?;
        Ok(json!({
            "exit_code": output.status.code().unwrap_or(-1),
            "stdout": String::from_utf8_lossy(&output.stdout),
            "stderr": String::from_utf8_lossy(&output.stderr),
            "shell": default_shell_name(),
            "cwd": cwd.display().to_string(),
        }))
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
