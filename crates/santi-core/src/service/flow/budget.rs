use serde_json::json;

use crate::context::budget::estimate_provider_parts;
use crate::service::tools::provider_tools;
use crate::store::budget::{Admission, Pressure};
use crate::{Fault, catalog};

use super::super::Service;
use crate::budget;

const REASON_PROVIDER: &str = "provider_request_exceeds_budget";

impl Service {
    pub(in crate::service) fn budget(&self) -> Option<budget::Cap> {
        self.provider.metadata().budget.map(|budget| budget::Cap {
            bytes: budget.bytes as i64,
            source: budget.source,
        })
    }

    pub(in crate::service) fn current_context_estimate(
        &self,
        strand: &str,
    ) -> Result<budget::Estimate, String> {
        let mut input = self.store.assembly_input(strand)?;
        input.extend(self.store.pending_provider_items(strand)?);
        let instructions = self.system_prompt_text(strand)?;
        let tools = provider_tools();
        Ok(estimate_provider_parts(
            &input,
            Some(&instructions),
            Some(&tools),
        ))
    }

    pub(in crate::service) fn context_admission(
        &self,
        strand: &str,
    ) -> Result<Option<Admission>, String> {
        let Some(budget) = self.budget() else {
            return Ok(None);
        };
        let metadata = self.provider.metadata();
        Ok(Some(Admission {
            provider: metadata.provider.to_string(),
            model: metadata.model,
            budget_source: budget.source,
            budget_bytes: budget.bytes,
            instructions: Some(self.system_prompt_text(strand)?),
            tools: provider_tools(),
        }))
    }

    pub(in crate::service) fn open_over_budget_incident(
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
        let reason = over_budget_reason(estimate.total, budget.bytes);
        let error = self.store.open_context_incident(
            &strand.id,
            Pressure {
                reason_code: REASON_PROVIDER,
                reason_text: &reason,
                operation: "provider_preflight",
                provider: Some(metadata.provider.as_ref()),
                model: Some(&request.model),
                budget_source: Some(&budget.source),
                budget_bytes: Some(budget.bytes),
                estimate,
                observed_turn_id: Some(turn),
                observed_at_seq: Some(strand.next - 1),
                metadata: Some(json!({
                    "phase": "provider_preflight",
                    "estimator": estimate.estimator,
                })),
            },
        )?;
        self.dispatch_error_events();
        Ok(Some(error))
    }

    pub(in crate::service) fn clear_context_incident(
        &self,
        strand: &str,
        cleared_by: &str,
    ) -> Result<bool, String> {
        if self.store.active_context_incident(strand)?.is_none() {
            return Ok(false);
        }
        let Some(budget) = self.budget() else {
            return Ok(false);
        };
        let estimate = self.current_context_estimate(strand)?;
        if estimate.total > budget.bytes {
            return Ok(false);
        }
        let resolved = self
            .store
            .resolve_context_incident(strand, cleared_by, &estimate)?;
        self.dispatch_error_events();
        Ok(resolved)
    }
}

fn over_budget_reason(total: i64, budget_bytes: i64) -> String {
    format!("strand context is over budget ({total} estimated bytes, budget {budget_bytes})")
}

pub(super) enum Verdict {
    Unbounded,
    Bounded(Vec<usize>),
    Rejected(Box<Fault>),
}

impl Service {
    pub(super) fn admit_execution_round(
        &self,
        strand: &str,
        turn: &str,
        next_round: usize,
    ) -> Result<Option<Fault>, String> {
        let Some(budget) = self.strand_execution_budget(strand) else {
            return Ok(None);
        };
        if next_round <= budget.rounds {
            return Ok(None);
        }
        let usage = self.store.strand_execution_usage(strand)?;
        self.open_execution_budget_incident(Breach {
            strand,
            turn,
            budget: &budget,
            usage,
            reason: "provider_rounds",
            request: json!({"next_provider_round": next_round}),
        })
        .map(Some)
    }

    pub(super) fn admit_tool_batch(
        &self,
        strand: &str,
        turn: &str,
        round: usize,
        call_count: usize,
    ) -> Result<Verdict, String> {
        let Some(budget) = self.strand_execution_budget(strand) else {
            return Ok(Verdict::Unbounded);
        };
        let usage = self.store.strand_execution_usage(strand)?;
        let request = json!({
            "provider_round": round,
            "calls": call_count,
        });
        if round >= budget.rounds {
            return self
                .open_execution_budget_incident(Breach {
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
        if usage.calls.saturating_add(call_count) > budget.calls {
            return self
                .open_execution_budget_incident(Breach {
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
        let output_remaining = budget.output.saturating_sub(usage.output);
        if output_remaining < call_count {
            return self
                .open_execution_budget_incident(Breach {
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
        Ok(Verdict::Bounded(allocate_capture_limits(
            output_remaining,
            budget.shell,
            call_count,
        )))
    }

    fn open_execution_budget_incident(&self, breach: Breach<'_>) -> Result<Fault, String> {
        let Breach {
            strand,
            turn,
            budget,
            usage,
            reason,
            request,
        } = breach;
        let error = self.store.open_error_incident(santi_error::Draft {
            key: crate::store::execution_budget_incident_key(strand),
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
        self.dispatch_error_events();
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

fn allocate_capture_limits(total: usize, per_call: usize, call_count: usize) -> Vec<usize> {
    let mut remaining = total;
    let mut limits = Vec::with_capacity(call_count);
    for index in 0..call_count {
        let remaining_calls = call_count - index;
        let limit = (remaining / remaining_calls).min(per_call);
        limits.push(limit);
        remaining -= limit;
    }
    limits
}
