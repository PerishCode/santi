use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use santi_error::Incident;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct ContextEstimate {
    pub estimator: String,
    pub items: i64,
    pub input: i64,
    pub instructions: i64,
    pub tools: i64,
    pub total: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct ContextBudget {
    pub bytes: i64,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StrandBudgetSnapshot {
    pub strand: String,
    pub estimate: ContextEstimate,
    pub budget: Option<ContextBudget>,
    pub incident: Option<Incident>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Execution {
    pub profile: String,
    pub rounds: usize,
    pub calls: usize,
    pub output: usize,
    pub shell: usize,
}

impl Execution {
    pub fn validate(&self) -> Result<(), String> {
        if self.profile.trim().is_empty() {
            return Err("execution budget profile must not be empty".to_string());
        }
        if self.rounds == 0 {
            return Err("execution budget rounds must be positive".to_string());
        }
        if self.calls == 0 {
            return Err("execution budget calls must be positive".to_string());
        }
        if self.output == 0 {
            return Err("execution budget output must be positive".to_string());
        }
        if self.shell == 0 {
            return Err("execution budget shell must be positive".to_string());
        }
        if self.shell > self.output {
            return Err("execution budget shell must not exceed output".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub calls: usize,
    pub output: usize,
}
