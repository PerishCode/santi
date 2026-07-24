use std::{path::PathBuf, process::Stdio};

use santi_provider::{Call, Function, Tool};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::workspace;
use crate::{SOULSPACE, STRANDSPACE, parsed, soulward, strandward};

use super::Service;
use crate::{effect, stream};

mod shell;

pub(crate) fn tools() -> Vec<Tool> {
    let soulward = soulward();
    let strandward = strandward();
    vec![Tool::Function(Function {
        name: "shell".to_string(),
        description: format!(
            "Run a shell command. By default commands run in the current execution workspace. Use cwd \"{SOULSPACE}\" to work in the current soul workspace, where {soulward} is always rendered live in [santi-soul]. Use cwd \"{STRANDSPACE}\" to work in the current strand workspace, where {strandward} is always rendered live in [santi-strand]. Unix-like systems use bash by default; Windows uses pwsh by default."
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
                    "description": format!("Optional workspace URI. Supports {SOULSPACE}, {SOULSPACE}<path>, {STRANDSPACE}, and {STRANDSPACE}<path>.")
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
    call: &'a Call,
    effect: &'a str,
    limit: Option<usize>,
}

impl Service {
    pub(super) fn tooled(
        &self,
        strand: &str,
        turn: &str,
        call: Call,
        output_limit: Option<usize>,
    ) -> Result<(), String> {
        let kind = (call.name == "shell").then_some("shell");
        let provenance = crate::tool::Provenance {
            family: self.provider.metadata().provider.to_string(),
            item: Some(call.item.clone()),
            mark: call.mark.clone(),
            response: Some(call.response.clone()),
        };
        let (held, effect) = self.store.charge(
            crate::Invocation {
                turn,
                call: &call.call,
                name: &call.name,
                arguments: &call.arguments,
                provenance: &provenance,
            },
            kind,
        )?;
        self.publish(
            strand,
            stream::Payload::ToolCallCreated { call: held.clone() },
        );
        let result = if let Some(effect) = effect {
            self.shelled(Shell {
                strand,
                turn,
                call: &call,
                effect: &effect.id,
                limit: output_limit,
            })?
        } else {
            self.store.reply(
                &call.call,
                None,
                Some(curbed(
                    format!("unsupported tool: {}", call.name),
                    output_limit,
                )),
            )?
        };
        self.publish(strand, stream::Payload::ToolResultCreated { result });
        Ok(())
    }

    fn shelled(&self, shell: Shell<'_>) -> Result<crate::tool::Reply, String> {
        let Shell {
            strand,
            turn,
            call,
            effect,
            limit: output_limit,
        } = shell;
        let soul = self.store.keeper(strand)?;
        let prepared = match argued::<shell::Args>(&call.arguments)
            .and_then(|args| self.prepared(strand, turn, &soul, args))
        {
            Ok(prepared) => prepared,
            Err(error) => {
                return self.store.redeem(
                    effect,
                    crate::store::Settlement {
                        call: &call.call,
                        output: None,
                        error: Some(curbed(error, output_limit)),
                        state: effect::State::Settled(effect::Outcome::NotApplied),
                    },
                );
            }
        };
        self.store.dispatch(effect)?;
        match shell::ran(prepared, output_limit) {
            shell::Outcome::Captured(output) => self.store.redeem(
                effect,
                crate::store::Settlement {
                    call: &call.call,
                    output: Some(output),
                    error: None,
                    state: effect::State::Settled(effect::Outcome::Applied),
                },
            ),
            shell::Outcome::Failed(error) => self.store.redeem(
                effect,
                crate::store::Settlement {
                    call: &call.call,
                    output: None,
                    error: Some(curbed(error, output_limit)),
                    state: effect::State::Settled(effect::Outcome::NotApplied),
                },
            ),
            shell::Outcome::Unknown(error) => {
                self.store.unmark(effect, &error)?;
                Err(format!(
                    "shell effect {effect} outcome is unknown; automatic replay is forbidden: {error}"
                ))
            }
        }
    }

    fn prepared(
        &self,
        strand: &str,
        turn: &str,
        soul: &str,
        args: shell::Args,
    ) -> Result<shell::Prepared, String> {
        std::fs::create_dir_all(self.soulhome(soul)).map_err(|error| error.to_string())?;
        std::fs::create_dir_all(self.strandhome(strand)).map_err(|error| error.to_string())?;
        let cwd = self.situated(strand, soul, args.cwd.as_deref())?;
        std::fs::create_dir_all(&cwd).map_err(|error| error.to_string())?;
        let mut command = shell::shell(&args.command);
        command
            .current_dir(&cwd)
            .env("SANTI_SOUL_MEMORY_DIR", self.soulhome(soul))
            .env("SANTI_STRAND_MEMORY_DIR", self.strandhome(strand))
            .env("SANTI_SOUL_ID", soul)
            .env("SANTI_STRAND_ID", strand)
            .env("SANTI_TURN_ID", turn)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        Ok(shell::Prepared { command, cwd })
    }

    fn situated(&self, strand: &str, soul: &str, cwd: Option<&str>) -> Result<PathBuf, String> {
        let Some(cwd) = cwd else {
            return Ok(self.execution());
        };
        let uri = parsed(cwd)?;
        let root = match uri.root {
            workspace::Root::Soul => self.soulhome(soul),
            workspace::Root::Strand => self.strandhome(strand),
        };
        Ok(root.join(uri.path))
    }

    pub(super) fn runtime(&self) -> PathBuf {
        PathBuf::from(&self.config.runtime)
    }

    pub(super) fn execution(&self) -> PathBuf {
        PathBuf::from(&self.config.execution)
    }

    pub(super) fn soulhome(&self, soul: &str) -> PathBuf {
        self.runtime().join("souls").join(soul).join("memory")
    }

    pub(super) fn memoir(&self, soul: &str) -> PathBuf {
        crate::store::memoir(self.runtime(), soul)
    }

    pub(super) fn strandhome(&self, strand: &str) -> PathBuf {
        self.runtime().join("strands").join(strand).join("memory")
    }

    pub(super) fn journal(&self, strand: &str) -> PathBuf {
        self.strandhome(strand).join("MEMORY.md")
    }

    pub(super) fn charter(&self) -> PathBuf {
        self.config
            .constitution
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.runtime().join("constitution.md"))
    }
}

fn curbed(error: String, limit: Option<usize>) -> String {
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

fn argued<T: for<'de> Deserialize<'de>>(value: &Value) -> Result<T, String> {
    serde_json::from_value(value.clone()).map_err(|error| error.to_string())
}
