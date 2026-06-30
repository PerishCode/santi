use std::{path::PathBuf, process::Command};

use santi_provider::ProviderFunctionCall;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{SantiStreamPayload, WorkspaceRoot, parse_workspace_uri};

use super::SantiService;

impl SantiService {
    pub(super) fn handle_tool_call(
        &self,
        session_id: &str,
        soul_session_id: &str,
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
            session_id,
            SantiStreamPayload::ToolCallCreated {
                tool_call: tool_call.clone(),
            },
        );
        let dispatch = self.dispatch_tool(session_id, soul_session_id, &call);
        let (output, error_text) = match dispatch {
            Ok(output) => (Some(output), None),
            Err(error) => (None, Some(error)),
        };
        let result = self
            .store
            .append_tool_result(&call.call_id, output, error_text)?;
        self.publish_stream(
            session_id,
            SantiStreamPayload::ToolResultCreated {
                tool_result: result,
            },
        );
        Ok(())
    }

    fn dispatch_tool(
        &self,
        session_id: &str,
        _soul_session_id: &str,
        call: &ProviderFunctionCall,
    ) -> Result<Value, String> {
        match call.name.as_str() {
            "shell" => {
                let args = parse_tool_args::<ShellArgs>(&call.arguments)?;
                self.run_shell(session_id, args)
            }
            name => Err(format!("unsupported tool: {name}")),
        }
    }

    fn run_shell(&self, session_id: &str, args: ShellArgs) -> Result<Value, String> {
        std::fs::create_dir_all(self.soul_memory_dir()).map_err(|error| error.to_string())?;
        std::fs::create_dir_all(self.session_memory_dir(session_id))
            .map_err(|error| error.to_string())?;
        let cwd = self.resolve_shell_cwd(session_id, args.cwd.as_deref())?;
        std::fs::create_dir_all(&cwd).map_err(|error| error.to_string())?;
        let mut command = shell_command(&args.command);
        let output = command
            .current_dir(&cwd)
            .env("SANTI_SOUL_MEMORY_DIR", self.soul_memory_dir())
            .env(
                "SANTI_SESSION_MEMORY_DIR",
                self.session_memory_dir(session_id),
            )
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

    fn resolve_shell_cwd(&self, session_id: &str, cwd: Option<&str>) -> Result<PathBuf, String> {
        let Some(cwd) = cwd else {
            return Ok(self.execution_root());
        };
        let uri = parse_workspace_uri(cwd)?;
        let root = match uri.root {
            WorkspaceRoot::Soul => self.soul_memory_dir(),
            WorkspaceRoot::Session => self.session_memory_dir(session_id),
        };
        Ok(root.join(uri.path))
    }

    pub(super) fn runtime_root(&self) -> PathBuf {
        PathBuf::from(&self.config.runtime_root)
    }

    pub(super) fn execution_root(&self) -> PathBuf {
        PathBuf::from(&self.config.execution_root)
    }

    pub(super) fn soul_memory_dir(&self) -> PathBuf {
        self.runtime_root().join("souls").join("memory")
    }

    pub(super) fn soul_memory_file(&self) -> PathBuf {
        self.soul_memory_dir().join("MEMORY.md")
    }

    pub(super) fn session_memory_dir(&self, session_id: &str) -> PathBuf {
        self.runtime_root()
            .join("sessions")
            .join(session_id)
            .join("memory")
    }

    pub(super) fn session_memory_file(&self, session_id: &str) -> PathBuf {
        self.session_memory_dir(session_id).join("MEMORY.md")
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
