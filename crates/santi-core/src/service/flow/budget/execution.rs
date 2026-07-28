use super::*;

pub(in crate::service) enum Verdict {
    Unbounded,
    Bounded(Vec<usize>),
    Rejected(Box<Fault>),
}

impl Service {
    pub(in crate::service) async fn readmit(
        &self,
        strand: &str,
        turn: &str,
        next: usize,
    ) -> Result<Option<Fault>, String> {
        let Some(budget) = self.rationed(strand) else {
            return Ok(None);
        };
        if next <= budget.rounds {
            return Ok(None);
        }
        let usage = self.usage(strand).await?;
        self.breached(Breach {
            strand,
            turn,
            budget: &budget,
            usage,
            reason: "provider_rounds",
            request: json!({"next_provider_round": next}),
        })
        .await
        .map(Some)
    }

    pub(in crate::service) async fn judge(
        &self,
        strand: &str,
        turn: &str,
        round: usize,
        calls: usize,
    ) -> Result<Verdict, String> {
        let Some(budget) = self.rationed(strand) else {
            return Ok(Verdict::Unbounded);
        };
        let usage = self.usage(strand).await?;
        let request = json!({
            "provider_round": round,
            "calls": calls,
        });
        if round >= budget.rounds {
            return self
                .breached(Breach {
                    strand,
                    turn,
                    budget: &budget,
                    usage,
                    reason: "provider_rounds",
                    request,
                })
                .await
                .map(Box::new)
                .map(Verdict::Rejected);
        }
        if usage.calls.saturating_add(calls) > budget.calls {
            return self
                .breached(Breach {
                    strand,
                    turn,
                    budget: &budget,
                    usage,
                    reason: "calls",
                    request,
                })
                .await
                .map(Box::new)
                .map(Verdict::Rejected);
        }
        let room = budget.output.saturating_sub(usage.output);
        if room < calls {
            return self
                .breached(Breach {
                    strand,
                    turn,
                    budget: &budget,
                    usage,
                    reason: "output",
                    request,
                })
                .await
                .map(Box::new)
                .map(Verdict::Rejected);
        }
        Ok(Verdict::Bounded(allotted(room, budget.shell, calls)))
    }

    async fn usage(&self, strand: &str) -> Result<budget::Usage, String> {
        let calls = self.store.calls(strand).await?;
        let results = self.store.results(strand).await?;
        let output = results.into_iter().fold(0usize, |held, result| {
            held.saturating_add(match result.output {
                Some(output) => captured(&output),
                None => result.error.as_deref().map_or(0, str::len),
            })
        });
        Ok(budget::Usage {
            calls: calls.len(),
            output,
        })
    }

    async fn breached(&self, breach: Breach<'_>) -> Result<Fault, String> {
        let Breach {
            strand,
            turn,
            budget,
            usage,
            reason,
            request,
        } = breach;
        let error = self
            .store
            .raise(
                santi_error::Draft {
                    key: crate::budget::Error::Execution
                        .descriptor()
                        .key("strand", strand),
                    descriptor: crate::budget::Error::Execution.descriptor(),
                    scope: santi_error::Scope::new("strand", strand),
                    source: santi_error::Source::new("santi-core", "turn.execution_budget"),
                    message: format!("strand execution budget exceeded: {reason}"),
                    context: json!({
                        "schema": "santi.error.execution_budget.v1",
                        "profile": budget.profile,
                        "reason": reason,
                        "turn": turn,
                        "limits": {
                            "provider_rounds": budget.rounds,
                            "calls": budget.calls,
                            "output": budget.output,
                            "shell_output_bytes": budget.shell,
                        },
                        "usage": {
                            "calls": usage.calls,
                            "output": usage.output,
                        },
                        "request": request,
                    }),
                },
                &crate::now(),
            )
            .await?;
        self.dispatched().await;
        Ok(error)
    }
}

struct Breach<'a> {
    strand: &'a str,
    turn: &'a str,
    budget: &'a budget::Execution,
    usage: budget::Usage,
    reason: &'a str,
    request: serde_json::Value,
}

fn allotted(total: usize, per_call: usize, calls: usize) -> Vec<usize> {
    let mut remaining = total;
    let mut limits = Vec::with_capacity(calls);
    for index in 0..calls {
        let left = calls - index;
        let limit = (remaining / left).min(per_call);
        limits.push(limit);
        remaining -= limit;
    }
    limits
}

fn captured(output: &serde_json::Value) -> usize {
    let stdout = output
        .get("stdout")
        .and_then(serde_json::Value::as_str)
        .map_or(0, str::len);
    let stderr = output
        .get("stderr")
        .and_then(serde_json::Value::as_str)
        .map_or(0, str::len);
    if output.get("stdout").is_some() || output.get("stderr").is_some() {
        stdout.saturating_add(stderr)
    } else {
        output.to_string().len()
    }
}
