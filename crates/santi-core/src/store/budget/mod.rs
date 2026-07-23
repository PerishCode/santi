mod inbox;
mod state;
mod turn;

pub(crate) use turn::execution_budget_incident_key;

use rusqlite::{OptionalExtension, params};
use santi_error::{Fault, Incident, catalog, engine};
use santi_provider::{ProviderItem, ProviderTool};
use serde_json::{Value, json};

use super::{
    STRAND_INBOX_GATE, SantiStore, StartTurnOutcome, StartedTurn,
    assembly::assembly_input_in_conn,
    db::{Database, drain_inbox_in_tx},
};
use crate::{budget, ingest, message};
use crate::{now, tag};

const REASON_PENDING: &str = "pending_drain_would_exceed_budget";

pub(crate) fn context_incident_key(strand: &str) -> String {
    format!("{}:strand:{strand}", catalog::CONTEXT_BUDGET_EXCEEDED.code)
}

pub(crate) struct Pressure<'a> {
    pub reason_code: &'a str,
    pub reason_text: &'a str,
    pub operation: &'a str,
    pub provider: Option<&'a str>,
    pub model: Option<&'a str>,
    pub budget_source: Option<&'a str>,
    pub budget_bytes: Option<i64>,
    pub estimate: &'a budget::Estimate,
    pub observed_turn_id: Option<&'a str>,
    pub observed_at_seq: Option<i64>,
    pub metadata: Option<Value>,
}

impl Pressure<'_> {
    fn into_draft(self, strand: &str) -> santi_error::Draft {
        santi_error::Draft {
            key: context_incident_key(strand),
            descriptor: catalog::CONTEXT_BUDGET_EXCEEDED,
            scope: santi_error::Scope::new("strand", strand),
            source: santi_error::Source::new("santi-core", self.operation),
            message: self.reason_text.to_string(),
            context: json!({
                "schema": "santi.error.context_budget.v1",
                "reason": self.reason_code,
                "provider": self.provider,
                "model": self.model,
                "budget": {
                    "source": self.budget_source,
                    "input": self.budget_bytes,
                },
                "estimate": self.estimate,
                "observed_turn_id": self.observed_turn_id,
                "observed_at_seq": self.observed_at_seq,
                "details": self.metadata,
            }),
        }
    }
}

pub(crate) struct Admission {
    pub provider: String,
    pub model: String,
    pub budget_source: String,
    pub budget_bytes: i64,
    pub instructions: Option<String>,
    pub tools: Vec<ProviderTool>,
}

pub(crate) struct Ingress<'a> {
    pub strand: &'a str,
    pub kind: message::Kind,
    pub content: message::Content,
    pub source: Option<ingest::Source>,
    pub admission: Option<&'a Admission>,
    pub replay: Option<Replay<'a>>,
}

pub(crate) struct Replay<'a> {
    pub owner: &'a str,
    pub request: &'a str,
    pub digest: &'a str,
}

pub(crate) struct Intake {
    pub outcome: ingest::Outcome,
    pub inserted: bool,
}

pub(crate) struct Launch<'a> {
    pub strand: &'a str,
    pub trigger: &'a str,
    pub reference: Option<&'a str>,
    pub admission: Option<&'a Admission>,
    pub recover: bool,
}

impl SantiStore {
    pub(crate) fn pending_provider_items(&self, strand: &str) -> Result<Vec<ProviderItem>, String> {
        let conn = self.conn.lock().unwrap();
        state::pending_items(&Database::new(&conn), strand)
    }

    pub(crate) fn open_context_incident(
        &self,
        strand: &str,
        input: Pressure<'_>,
    ) -> Result<Fault, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let error = state::open_context_incident(&Database::new(&tx), strand, input)?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(error)
    }

    pub(crate) fn active_context_incident(&self, strand: &str) -> Result<Option<Incident>, String> {
        self.active_error_incident(&context_incident_key(strand))
    }

    pub(crate) fn resolve_context_incident(
        &self,
        strand: &str,
        resolved_by: &str,
        estimate: &budget::Estimate,
    ) -> Result<bool, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let resolved = Database::new(&tx).resolve_incident(
            &context_incident_key(strand),
            resolved_by,
            json!({
                "schema": "santi.error.context_budget.resolution.v1",
                "resolved_by": resolved_by,
                "estimate": estimate,
            }),
        )?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(resolved)
    }
}

pub(super) fn over_budget_reason(total: i64, budget_bytes: i64) -> String {
    format!("strand context is over budget ({total} estimated bytes, budget {budget_bytes})")
}
