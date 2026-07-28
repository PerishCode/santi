use santi_provider::{Call, Function, Tool};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::service::address::Address;
use crate::{SOULSPACE, STRANDSPACE, soulward, strandward};

use super::Service;
use crate::service::interrupt::Control;
use crate::{effect, stream};

mod shell;
mod workspace;

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

struct Origin<'a> {
    strand: &'a str,
    turn: &'a str,
    soul: &'a str,
    call: &'a str,
    effect: &'a str,
}

impl Service {
    pub(super) async fn tooled(
        &self,
        address: Address<&str>,
        call: Call,
        output_limit: Option<usize>,
        control: &Control,
    ) -> Result<(), String> {
        let Address { strand, turn } = address;
        let kind = (call.name == "shell").then_some("shell");
        let created = crate::now();
        let effect_tag = kind.map(|_| crate::tag("effect"));
        let (held, effect) = self
            .store
            .prepare_invocation(
                santi_estate::CallDraft {
                    tag: &call.call,
                    turn,
                    tool: &call.name,
                    arguments: &call.arguments,
                    created: &created,
                },
                effect_tag.as_deref().map(|tag| santi_estate::EffectDraft {
                    tag,
                    turn,
                    call: Some(&call.call),
                    kind: kind.expect("effect kind"),
                    metadata: None,
                    created: &created,
                }),
            )
            .await?;
        self.publish(
            strand,
            stream::Payload::Tool(crate::tool::Beat::Called { call: held.clone() }),
        );
        let result = if let Some(effect) = effect {
            self.shelled(
                Shell {
                    strand,
                    turn,
                    call: &call,
                    effect: &effect.id,
                    limit: output_limit,
                },
                control,
            )
            .await?
        } else {
            let error = curbed(format!("unsupported tool: {}", call.name), output_limit);
            self.store
                .create_reply(santi_estate::ReplyDraft {
                    tag: &crate::tag("result"),
                    call: &call.call,
                    output: None,
                    error: Some(&error),
                    created: &crate::now(),
                })
                .await?
        };
        self.publish(
            strand,
            stream::Payload::Tool(crate::tool::Beat::Replied { result }),
        );
        Ok(())
    }

    async fn shelled(
        &self,
        shell: Shell<'_>,
        control: &Control,
    ) -> Result<crate::tool::Reply, String> {
        let Shell {
            strand,
            turn,
            call,
            effect,
            limit: output_limit,
        } = shell;
        let soul = self
            .store
            .strand(strand)
            .await?
            .map(|strand| strand.soul)
            .ok_or_else(|| "strand not found".to_string())?;
        let preparation = match argued::<shell::Args>(&call.arguments) {
            Ok(args) => {
                self.prepared(
                    Origin {
                        strand,
                        turn,
                        soul: &soul,
                        call: &call.call,
                        effect,
                    },
                    args,
                )
                .await
            }
            Err(error) => Err(error),
        };
        let prepared = match preparation {
            Ok(prepared) => prepared,
            Err(error) => {
                let error = curbed(error, output_limit);
                return self
                    .store
                    .redeem_effect(
                        effect,
                        santi_estate::RedemptionDraft {
                            result: &crate::tag("result"),
                            call: &call.call,
                            output: None,
                            error: Some(&error),
                            outcome: effect::Outcome::NotApplied,
                            occurred: &crate::now(),
                        },
                    )
                    .await;
            }
        };
        if let Some(cause) = self.halted(control) {
            let error = format!("interrupted by {} before dispatch", cause.encode());
            return self
                .store
                .redeem_effect(
                    effect,
                    santi_estate::RedemptionDraft {
                        result: &crate::tag("result"),
                        call: &call.call,
                        output: None,
                        error: Some(&error),
                        outcome: effect::Outcome::NotApplied,
                        occurred: &crate::now(),
                    },
                )
                .await;
        }
        self.store.dispatch_effect(effect, &crate::now()).await?;
        match shell::ran(prepared, output_limit, control).await {
            shell::Outcome::Captured(output) => {
                self.store
                    .redeem_effect(
                        effect,
                        santi_estate::RedemptionDraft {
                            result: &crate::tag("result"),
                            call: &call.call,
                            output: Some(&output),
                            error: None,
                            outcome: effect::Outcome::Applied,
                            occurred: &crate::now(),
                        },
                    )
                    .await
            }
            shell::Outcome::Failed(error) => {
                let error = curbed(error, output_limit);
                self.store
                    .redeem_effect(
                        effect,
                        santi_estate::RedemptionDraft {
                            result: &crate::tag("result"),
                            call: &call.call,
                            output: None,
                            error: Some(&error),
                            outcome: effect::Outcome::NotApplied,
                            occurred: &crate::now(),
                        },
                    )
                    .await
            }
            shell::Outcome::Unknown(error) => {
                self.store
                    .unknown_effect(effect, &error, &crate::now())
                    .await?;
                Err(format!(
                    "shell effect {effect} outcome is unknown; automatic replay is forbidden: {error}"
                ))
            }
            shell::Outcome::Stopped(error) => {
                self.store
                    .unknown_effect(effect, &error, &crate::now())
                    .await?;
                Err(error)
            }
        }
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
