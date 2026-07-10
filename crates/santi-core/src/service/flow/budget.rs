use serde_json::json;

use crate::context_budget::estimate_provider_parts;
use crate::service_prompt::provider_tools;
use crate::store::budget::{ContextAdmission, ContextIncidentInput};
use crate::{ContextBudget, ContextEstimate, SantiError};

use super::super::SantiService;

const REASON_PROVIDER: &str = "provider_request_exceeds_budget";

impl SantiService {
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
    ) -> Result<Option<ContextAdmission>, String> {
        let Some(budget) = self.context_budget() else {
            return Ok(None);
        };
        let metadata = self.provider.metadata();
        Ok(Some(ContextAdmission {
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
            ContextIncidentInput {
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
