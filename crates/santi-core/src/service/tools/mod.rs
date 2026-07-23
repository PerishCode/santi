use std::{path::PathBuf, process::Stdio};

use santi_provider::{ProviderFunctionCall, ProviderFunctionTool, ProviderTool};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::workspace;
use crate::{
    SOUL_WORKSPACE_URI, STRAND_WORKSPACE_URI, parse_workspace_uri, soul_memory_uri,
    strand_memory_uri,
};

use super::Service;
use crate::{effect, stream};

mod shell;

pub(crate) fn provider_tools() -> Vec<ProviderTool> {
    let soul_memory_uri = soul_memory_uri();
    let strand_memory_uri = strand_memory_uri();
    vec![ProviderTool::Function(ProviderFunctionTool {
        name: "shell".to_string(),
        description: format!(
            "Run a shell command. By default commands run in the current execution workspace. Use cwd \"{SOUL_WORKSPACE_URI}\" to work in the current soul workspace, where {soul_memory_uri} is always rendered live in [santi-soul]. Use cwd \"{STRAND_WORKSPACE_URI}\" to work in the current strand workspace, where {strand_memory_uri} is always rendered live in [santi-strand]. Unix-like systems use bash by default; Windows uses pwsh by default."
        ),
        parameters: json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute."
                },
                "cwd": {
                    "type": "string",
                    "description": format!("Optional workspace URI. Supports {SOUL_WORKSPACE_URI}, {SOUL_WORKSPACE_URI}<path>, {STRAND_WORKSPACE_URI}, and {STRAND_WORKSPACE_URI}<path>.")
                }
            },
            "required": ["command"],
            "additionalProperties": false
        }),
    })]
}

struct Shell<'a> {
    strand: &'a str,
    turn: &'a str,
    call: &'a ProviderFunctionCall,
    effect: &'a str,
    limit: Option<usize>,
}

impl Service {
    pub(super) fn handle_tool_call(
        &self,
        strand: &str,
        turn: &str,
        call: ProviderFunctionCall,
        output_limit: Option<usize>,
    ) -> Result<(), String> {
        let kind = (call.name == "shell").then_some("shell");
        let provenance = crate::tool::Provenance {
            family: self.provider.metadata().provider.to_string(),
            item: Some(call.item.clone()),
            mark: call.mark.clone(),
            response_id: Some(call.response_id.clone()),
        };
        let (tool_call, effect) = self.store.append_effect_call(
            crate::Invocation {
                turn,
                call: &call.call_id,
                name: &call.name,
                arguments: &call.arguments,
                provenance: &provenance,
            },
            kind,
        )?;
        self.publish_stream(
            strand,
            stream::Payload::ToolCallCreated {
                tool_call: tool_call.clone(),
            },
        );
        let result = if let Some(effect) = effect {
            self.handle_shell_effect(Shell {
                strand,
                turn,
                call: &call,
                effect: &effect.id,
                limit: output_limit,
            })?
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
            strand,
            stream::Payload::ToolResultCreated {
                tool_result: result,
            },
        );
        Ok(())
    }

    fn handle_shell_effect(&self, shell: Shell<'_>) -> Result<crate::tool::Reply, String> {
        let Shell {
            strand,
            turn,
            call,
            effect: effect_id,
            limit: output_limit,
        } = shell;
        let soul = self.store.soul_id_for_strand(strand)?;
        let prepared = match parse_tool_args::<shell::Args>(&call.arguments)
            .and_then(|args| self.prepare_shell(strand, turn, &soul, args))
        {
            Ok(prepared) => prepared,
            Err(error) => {
                return self.store.append_effect_tool_result(
                    effect_id,
                    crate::store::Settlement {
                        call: &call.call_id,
                        output: None,
                        error: Some(bounded_tool_error(error, output_limit)),
                        state: effect::State::NotDispatched,
                    },
                );
            }
        };
        self.store.begin_effect_dispatch(effect_id)?;
        match shell::run_prepared_shell(prepared, output_limit) {
            shell::Outcome::Captured(output) => self.store.append_effect_tool_result(
                effect_id,
                crate::store::Settlement {
                    call: &call.call_id,
                    output: Some(output),
                    error: None,
                    state: effect::State::Confirmed,
                },
            ),
            shell::Outcome::Failed(error) => self.store.append_effect_tool_result(
                effect_id,
                crate::store::Settlement {
                    call: &call.call_id,
                    output: None,
                    error: Some(bounded_tool_error(error, output_limit)),
                    state: effect::State::NotDispatched,
                },
            ),
            shell::Outcome::Unknown(error) => {
                self.store.mark_effect_unknown(
                    effect_id,
                    effect::Reason::ResultCaptureFailed,
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
        strand: &str,
        turn: &str,
        soul: &str,
        args: shell::Args,
    ) -> Result<shell::Prepared, String> {
        std::fs::create_dir_all(self.soul_memory_dir(soul)).map_err(|error| error.to_string())?;
        std::fs::create_dir_all(self.strand_memory_dir(strand))
            .map_err(|error| error.to_string())?;
        let cwd = self.resolve_shell_cwd(strand, soul, args.cwd.as_deref())?;
        std::fs::create_dir_all(&cwd).map_err(|error| error.to_string())?;
        let mut command = shell::shell_command(&args.command);
        command
            .current_dir(&cwd)
            .env("SANTI_SOUL_MEMORY_DIR", self.soul_memory_dir(soul))
            .env("SANTI_STRAND_MEMORY_DIR", self.strand_memory_dir(strand))
            .env("SANTI_SOUL_ID", soul)
            .env("SANTI_STRAND_ID", strand)
            .env("SANTI_TURN_ID", turn)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        Ok(shell::Prepared { command, cwd })
    }

    fn resolve_shell_cwd(
        &self,
        strand: &str,
        soul: &str,
        cwd: Option<&str>,
    ) -> Result<PathBuf, String> {
        let Some(cwd) = cwd else {
            return Ok(self.execution_root());
        };
        let uri = parse_workspace_uri(cwd)?;
        let root = match uri.root {
            workspace::Root::Soul => self.soul_memory_dir(soul),
            workspace::Root::Strand => self.strand_memory_dir(strand),
        };
        Ok(root.join(uri.path))
    }

    pub(super) fn runtime_root(&self) -> PathBuf {
        PathBuf::from(&self.config.runtime_root)
    }

    pub(super) fn execution_root(&self) -> PathBuf {
        PathBuf::from(&self.config.execution_root)
    }

    pub(super) fn soul_memory_dir(&self, soul: &str) -> PathBuf {
        self.runtime_root().join("souls").join(soul).join("memory")
    }

    pub(super) fn soul_memory_file(&self, soul: &str) -> PathBuf {
        crate::store::soul_memory_file(self.runtime_root(), soul)
    }

    pub(super) fn strand_memory_dir(&self, strand: &str) -> PathBuf {
        self.runtime_root()
            .join("strands")
            .join(strand)
            .join("memory")
    }

    pub(super) fn strand_memory_file(&self, strand: &str) -> PathBuf {
        self.strand_memory_dir(strand).join("MEMORY.md")
    }

    pub(super) fn constitution_file(&self) -> PathBuf {
        self.config
            .constitution_path
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.runtime_root().join("constitution.md"))
    }
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

fn parse_tool_args<T: for<'de> Deserialize<'de>>(value: &Value) -> Result<T, String> {
    serde_json::from_value(value.clone()).map_err(|error| error.to_string())
}
