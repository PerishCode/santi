use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::ErrorIncident;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct ContextEstimate {
    pub estimator: String,
    pub input_items: i64,
    pub input_bytes: i64,
    pub instructions_bytes: i64,
    pub tools_bytes: i64,
    pub total_bytes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct ContextBudget {
    pub input_budget_bytes: i64,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StrandBudgetSnapshot {
    pub strand_id: String,
    pub estimate: ContextEstimate,
    pub budget: Option<ContextBudget>,
    pub active_incident: Option<ErrorIncident>,
}

/// A runtime-enforced envelope for one strand. The boundary adaptor decides
/// which strands receive a budget; core only enforces the registered limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrandExecutionBudget {
    pub profile: String,
    pub max_provider_rounds: usize,
    pub max_tool_calls: usize,
    pub max_tool_output_bytes: usize,
    pub max_shell_output_bytes: usize,
}

impl StrandExecutionBudget {
    pub fn validate(&self) -> Result<(), String> {
        if self.profile.trim().is_empty() {
            return Err("execution budget profile must not be empty".to_string());
        }
        if self.max_provider_rounds == 0 {
            return Err("execution budget max_provider_rounds must be positive".to_string());
        }
        if self.max_tool_calls == 0 {
            return Err("execution budget max_tool_calls must be positive".to_string());
        }
        if self.max_tool_output_bytes == 0 {
            return Err("execution budget max_tool_output_bytes must be positive".to_string());
        }
        if self.max_shell_output_bytes == 0 {
            return Err("execution budget max_shell_output_bytes must be positive".to_string());
        }
        if self.max_shell_output_bytes > self.max_tool_output_bytes {
            return Err(
                "execution budget max_shell_output_bytes must not exceed max_tool_output_bytes"
                    .to_string(),
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StrandExecutionUsage {
    pub tool_calls: usize,
    pub tool_output_bytes: usize,
}
