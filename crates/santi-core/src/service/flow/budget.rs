use serde_json::json;

use crate::context::budget::estimated;
use crate::service::tools::tools;
use crate::store::budget::{Admission, Pressure};
use crate::{Fault, catalog};

use super::super::Service;
use crate::budget;

const PROVIDER: &str = "provider_request_exceeds_budget";

impl Service {
    pub(in crate::service) fn budget(&self) -> Option<budget::Cap> {
        self.provider.metadata().budget.map(|budget| budget::Cap {
            bytes: budget.bytes as i64,
            source: budget.source,
        })
    }

    pub(in crate::service) fn estimate(&self, strand: &str) -> Result<budget::Estimate, String> {
        let mut input = self.store.assembly(strand)?;
        input.extend(self.store.pending(strand)?);
        let instructions = self.wording(strand)?;
        let tools = tools();
        Ok(estimated(&input, Some(&instructions), Some(&tools)))
    }

    pub(in crate::service) fn admission(&self, strand: &str) -> Result<Option<Admission>, String> {
        let Some(budget) = self.budget() else {
            return Ok(None);
        };
        let metadata = self.provider.metadata();
        Ok(Some(Admission {
            provider: metadata.provider.to_string(),
            model: metadata.model,
            source: budget.source,
            bytes: budget.bytes,
            instructions: Some(self.wording(strand)?),
            tools: tools(),
        }))
    }

    pub(in crate::service) fn overdrawn(
        &self,
        strand: &str,
        turn: &str,
        request: &santi_provider::Request,
        estimate: &budget::Estimate,
    ) -> Result<Option<Fault>, String> {
        let Some(budget) = self.budget() else {
            return Ok(None);
        };
        if estimate.total <= budget.bytes {
            return Ok(None);
        }
        let metadata = self.provider.metadata();
        let strand = self
            .store
            .strand(strand)?
            .ok_or_else(|| "strand not found".to_string())?;
        let reason = reason(estimate.total, budget.bytes);
        let error = self.store.press(
            &strand.id,
            Pressure {
                code: PROVIDER,
                text: &reason,
                operation: "provider_preflight",
                provider: Some(metadata.provider.as_ref()),
                model: Some(&request.model),
                source: Some(&budget.source),
                bytes: Some(budget.bytes),
                estimate,
                observed: Some(turn),
                at: Some(strand.next - 1),
                metadata: Some(json!({
                    "phase": "provider_preflight",
                    "estimator": estimate.estimator,
                })),
            },
        )?;
        self.dispatched();
        Ok(Some(error))
    }

    pub(in crate::service) fn absolve(
        &self,
        strand: &str,
        cleared_by: &str,
    ) -> Result<bool, String> {
        if self.store.pressure(strand)?.is_none() {
            return Ok(false);
        }
        let Some(budget) = self.budget() else {
            return Ok(false);
        };
        let estimate = self.estimate(strand)?;
        if estimate.total > budget.bytes {
            return Ok(false);
        }
        let resolved = self.store.vent(strand, cleared_by, &estimate)?;
        self.dispatched();
        Ok(resolved)
    }
}

fn reason(total: i64, bytes: i64) -> String {
    format!("strand context is over budget ({total} estimated bytes, budget {bytes})")
}

pub(super) enum Verdict {
    Unbounded,
    Bounded(Vec<usize>),
    Rejected(Box<Fault>),
}

impl Service {
    pub(super) fn readmit(
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
        let usage = self.store.spent(strand)?;
        self.breached(Breach {
            strand,
            turn,
            budget: &budget,
            usage,
            reason: "provider_rounds",
            request: json!({"next_provider_round": next}),
        })
        .map(Some)
    }

    pub(super) fn judge(
        &self,
        strand: &str,
        turn: &str,
        round: usize,
        calls: usize,
    ) -> Result<Verdict, String> {
        let Some(budget) = self.rationed(strand) else {
            return Ok(Verdict::Unbounded);
        };
        let usage = self.store.spent(strand)?;
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
                .map(Box::new)
                .map(Verdict::Rejected);
        }
        Ok(Verdict::Bounded(allotted(room, budget.shell, calls)))
    }

    fn breached(&self, breach: Breach<'_>) -> Result<Fault, String> {
        let Breach {
            strand,
            turn,
            budget,
            usage,
            reason,
            request,
        } = breach;
        let error = self.store.raise(santi_error::Draft {
            key: catalog::EXECUTION_BUDGET_EXCEEDED.key("strand", strand),
            descriptor: catalog::EXECUTION_BUDGET_EXCEEDED,
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
        })?;
        self.dispatched();
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
