mod inbox;
pub(in crate::store) mod notice;
mod state;
mod turn;

use crate::Ruled;
use rusqlite::{OptionalExtension, params};
use santi_error::{Fault, Incident, engine};
use santi_provider::{Item, Tool};
use serde_json::{Value, json};

use super::{
    Begun, GATE, Opened, Store,
    assembly::assembled,
    db::{Database, drain},
};
use crate::{budget, ingest, message};
use crate::{now, tag};

const PENDING: &str = "pending_drain_would_exceed_budget";

pub(crate) struct Pressure<'a> {
    pub code: &'a str,
    pub text: &'a str,
    pub operation: &'a str,
    pub provider: Option<&'a str>,
    pub model: Option<&'a str>,
    pub source: Option<&'a str>,
    pub bytes: Option<i64>,
    pub estimate: &'a budget::Estimate,
    pub observed: Option<&'a str>,
    pub at: Option<i64>,
    pub metadata: Option<Value>,
}

impl Pressure<'_> {
    fn drafted(self, strand: &str) -> santi_error::Draft {
        santi_error::Draft {
            key: crate::budget::Error::Context
                .descriptor()
                .key("strand", strand),
            descriptor: crate::budget::Error::Context.descriptor(),
            scope: santi_error::Scope::new("strand", strand),
            source: santi_error::Source::new("santi-core", self.operation),
            message: self.text.to_string(),
            context: json!({
                "schema": "santi.error.context_budget.v1",
                "reason": self.code,
                "provider": self.provider,
                "model": self.model,
                "budget": {
                    "source": self.source,
                    "input": self.bytes,
                },
                "estimate": self.estimate,
                "observed_turn_id": self.observed,
                "observed_at_seq": self.at,
                "details": self.metadata,
            }),
        }
    }
}

pub(crate) struct Admission {
    pub provider: String,
    pub model: String,
    pub source: String,
    pub bytes: i64,
    pub instructions: Option<String>,
    pub tools: Vec<Tool>,
}

pub(crate) struct Ingress<'a> {
    pub strand: &'a str,
    pub kind: message::Kind,
    pub content: message::Content,
    pub source: Option<ingest::Source>,
    pub admission: Option<&'a Admission>,
    pub replay: Option<Replay<'a>>,
}

pub(crate) enum Replay<'a> {
    Downstream {
        owner: &'a str,
        request: &'a str,
        digest: &'a str,
    },
    Webhook {
        subscription: &'a str,
        delivery: &'a str,
        digest: &'a str,
    },
}

pub(crate) struct Intake {
    pub outcome: ingest::Outcome,
    pub inserted: bool,
}

pub(crate) struct Notice<'a> {
    pub strand: &'a str,
    pub key: &'a str,
    pub revision: u64,
    pub digest: &'a str,
    pub content: message::Content,
    pub source: ingest::Source,
    pub causes: Vec<String>,
}

pub(crate) struct Offered {
    pub inbox: Option<String>,
    pub inserted: bool,
}

pub(crate) struct Launch<'a> {
    pub strand: &'a str,
    pub trigger: &'a str,
    pub reference: Option<&'a str>,
    pub admission: Option<&'a Admission>,
    pub recover: bool,
}

impl Store {
    pub(crate) fn pending(&self, strand: &str) -> Result<Vec<Item>, String> {
        let conn = self.conn.lock().unwrap();
        state::queued(&Database::new(&conn), strand)
    }

    pub(crate) fn press(&self, strand: &str, input: Pressure<'_>) -> Result<Fault, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let error = state::press(&Database::new(&tx), strand, input)?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(error)
    }

    pub(crate) fn pressure(&self, strand: &str) -> Result<Option<Incident>, String> {
        self.incident(
            &crate::budget::Error::Context
                .descriptor()
                .key("strand", strand),
        )
    }

    pub(crate) fn vent(
        &self,
        strand: &str,
        resolved_by: &str,
        estimate: &budget::Estimate,
    ) -> Result<bool, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let resolved = Database::new(&tx).resolve(
            &crate::budget::Error::Context
                .descriptor()
                .key("strand", strand),
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

pub(super) fn reason(total: i64, bytes: i64) -> String {
    format!("strand context is over budget ({total} estimated bytes, budget {bytes})")
}
