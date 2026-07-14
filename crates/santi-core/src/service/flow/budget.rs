use serde_json::json;

use crate::context::budget::estimate_provider_parts;
use crate::service::tools::provider_tools;
use crate::store::budget::{Admission, Pressure};
use crate::{
    ContextBudget, ContextEstimate, ErrorScope, ErrorSource, Execution, IncidentDraft, SantiError,
    Usage, catalog,
};

use super::super::Service;

const REASON_PROVIDER: &str = "provider_request_exceeds_budget";

impl Service {
    pub(in crate::service) fn context_budget(&self) -> Option<ContextBudget> {
        self.provider
            .metadata()
            .context_budget
            .map(|budget| ContextBudget {
                input_budget_bytes: budget.input_budget_bytes as i64,
                source: budget.source,
            })
    }

    pub(in crate::service) fn current_context_estimate(
        &self,
        strand_id: &str,
    ) -> Result<ContextEstimate, String> {
        let mut input = self.store.assembly_input(strand_id)?;
        input.extend(self.store.pending_provider_items(strand_id)?);
        let instructions = self.system_prompt_text(strand_id)?;
        let tools = provider_tools();
        Ok(estimate_provider_parts(
            &input,
            Some(&instructions),
            Some(&tools),
        ))
    }

    pub(in crate::service) fn context_admission(
        &self,
        strand_id: &str,
    ) -> Result<Option<Admission>, String> {
        let Some(budget) = self.context_budget() else {
            return Ok(None);
        };
        let metadata = self.provider.metadata();
        Ok(Some(Admission {
            provider: metadata.provider.to_string(),
            model: metadata.model,
            budget_source: budget.source,
            budget_bytes: budget.input_budget_bytes,
            instructions: Some(self.system_prompt_text(strand_id)?),
            tools: provider_tools(),
        }))
    }

    pub(in crate::service) fn open_over_budget_incident(
        &self,
        strand_id: &str,
        turn_id: &str,
        request: &santi_provider::ProviderRequest,
        estimate: &ContextEstimate,
    ) -> Result<Option<SantiError>, String> {
        let Some(budget) = self.context_budget() else {
            return Ok(None);
        };
        if estimate.total_bytes <= budget.input_budget_bytes {
            return Ok(None);
        }
        let metadata = self.provider.metadata();
        let strand = self
            .store
            .strand(strand_id)?
            .ok_or_else(|| "strand not found".to_string())?;
        let reason = over_budget_reason(estimate.total_bytes, budget.input_budget_bytes);
        let error = self.store.open_context_incident(
            strand_id,
            Pressure {
                reason_code: REASON_PROVIDER,
                reason_text: &reason,
                operation: "provider_preflight",
                provider: Some(metadata.provider.as_ref()),
                model: Some(&request.model),
                budget_source: Some(&budget.source),
                budget_bytes: Some(budget.input_budget_bytes),
                estimate,
                observed_turn_id: Some(turn_id),
                observed_at_seq: Some(strand.next_seq - 1),
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
        strand_id: &str,
        cleared_by: &str,
    ) -> Result<bool, String> {
        if self.store.active_context_incident(strand_id)?.is_none() {
            return Ok(false);
        }
        let Some(budget) = self.context_budget() else {
            return Ok(false);
        };
        let estimate = self.current_context_estimate(strand_id)?;
        if estimate.total_bytes > budget.input_budget_bytes {
            return Ok(false);
        }
        let resolved = self
            .store
            .resolve_context_incident(strand_id, cleared_by, &estimate)?;
        self.dispatch_error_events();
        Ok(resolved)
    }
}

fn over_budget_reason(total_bytes: i64, budget_bytes: i64) -> String {
    format!("strand context is over budget ({total_bytes} estimated bytes, budget {budget_bytes})")
}

pub(super) enum Verdict {
    Unbounded,
    Bounded(Vec<usize>),
    Rejected(Box<SantiError>),
}

impl Service {
    pub(super) fn admit_execution_round(
        &self,
        strand_id: &str,
        turn_id: &str,
        next_round: usize,
    ) -> Result<Option<SantiError>, String> {
        let Some(budget) = self.strand_execution_budget(strand_id) else {
            return Ok(None);
        };
        if next_round <= budget.max_provider_rounds {
            return Ok(None);
        }
        let usage = self.store.strand_execution_usage(strand_id)?;
        self.open_execution_budget_incident(Breach {
            strand: strand_id,
            turn: turn_id,
            budget: &budget,
            usage,
            reason: "provider_rounds",
            request: json!({"next_provider_round": next_round}),
        })
        .map(Some)
    }

    pub(super) fn admit_tool_batch(
        &self,
        strand_id: &str,
        turn_id: &str,
        round: usize,
        call_count: usize,
    ) -> Result<Verdict, String> {
        let Some(budget) = self.strand_execution_budget(strand_id) else {
            return Ok(Verdict::Unbounded);
        };
        let usage = self.store.strand_execution_usage(strand_id)?;
        let request = json!({
            "provider_round": round,
            "tool_calls": call_count,
        });
        if round >= budget.max_provider_rounds {
            return self
                .open_execution_budget_incident(Breach {
                    strand: strand_id,
                    turn: turn_id,
                    budget: &budget,
                    usage,
                    reason: "provider_rounds",
                    request,
                })
                .map(Box::new)
                .map(Verdict::Rejected);
        }
        if usage.tool_calls.saturating_add(call_count) > budget.max_tool_calls {
            return self
                .open_execution_budget_incident(Breach {
                    strand: strand_id,
                    turn: turn_id,
                    budget: &budget,
                    usage,
                    reason: "tool_calls",
                    request,
                })
                .map(Box::new)
                .map(Verdict::Rejected);
        }
        let output_remaining = budget
            .max_tool_output_bytes
            .saturating_sub(usage.tool_output_bytes);
        if output_remaining < call_count {
            return self
                .open_execution_budget_incident(Breach {
                    strand: strand_id,
                    turn: turn_id,
                    budget: &budget,
                    usage,
                    reason: "tool_output_bytes",
                    request,
                })
                .map(Box::new)
                .map(Verdict::Rejected);
        }
        Ok(Verdict::Bounded(allocate_capture_limits(
            output_remaining,
            budget.max_shell_output_bytes,
            call_count,
        )))
    }

    fn open_execution_budget_incident(&self, breach: Breach<'_>) -> Result<SantiError, String> {
        let Breach {
            strand,
            turn,
            budget,
            usage,
            reason,
            request,
        } = breach;
        let error = self.store.open_error_incident(IncidentDraft {
            incident_key: crate::store::execution_budget_incident_key(strand),
            descriptor: catalog::EXECUTION_BUDGET_EXCEEDED,
            scope: ErrorScope::new("strand", strand),
            source: ErrorSource::new("santi-core", "turn.execution_budget"),
            message: format!("strand execution budget exceeded: {reason}"),
            context: json!({
                "schema": "santi.error.execution_budget.v1",
                "profile": budget.profile,
                "reason": reason,
                "turn_id": turn,
                "limits": {
                    "provider_rounds": budget.max_provider_rounds,
                    "tool_calls": budget.max_tool_calls,
                    "tool_output_bytes": budget.max_tool_output_bytes,
                    "shell_output_bytes": budget.max_shell_output_bytes,
                },
                "usage": {
                    "tool_calls": usage.tool_calls,
                    "tool_output_bytes": usage.tool_output_bytes,
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
    budget: &'a Execution,
    usage: Usage,
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
