use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

pub mod catalog;
mod codec;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[schema(as = error::Category)]
pub enum Category {
    Internal,
    Invalid,
    Missing,
    Exhausted,
    Unauthorized,
    Unavailable,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[schema(as = error::Severity)]
pub enum Severity {
    Error,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[schema(as = error::Retry)]
pub enum Retry {
    Never,
    Later,
    Changed,
    Resolved,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[schema(as = error::Exposure)]
pub struct Exposure {
    pub caller: bool,
    pub operator: bool,
    pub model: bool,
}

impl Exposure {
    pub const CALLER_AND_OPERATOR: Self = Self {
        caller: true,
        operator: true,
        model: false,
    };
    pub const OPERATOR_ONLY: Self = Self {
        caller: false,
        operator: true,
        model: false,
    };
    pub const ALL: Self = Self {
        caller: true,
        operator: true,
        model: true,
    };
}

#[derive(Debug, Clone, Copy)]
pub struct Descriptor {
    pub code: &'static str,
    pub category: Category,
    pub severity: Severity,
    pub retry: Retry,
    pub exposure: Exposure,
}

impl Descriptor {
    pub fn key(&self, kind: &str, id: &str) -> String {
        format!("{}:{kind}:{id}", self.code)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[schema(as = error::Scope)]
pub struct Scope {
    pub kind: String,
    pub id: String,
}

impl Scope {
    pub fn new(kind: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            id: id.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[schema(as = error::Source)]
pub struct Source {
    pub component: String,
    pub operation: String,
}

impl Source {
    pub fn new(component: impl Into<String>, operation: impl Into<String>) -> Self {
        Self {
            component: component.into(),
            operation: operation.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[schema(as = error::Status)]
pub enum Status {
    Active,
    Resolved,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = error::Report)]
pub struct Report {
    pub source: Source,
    pub message: String,
    pub context: Value,
    pub seen: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = error::Resolution)]
pub struct Resolution {
    pub at: String,
    pub by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = error::Incident)]
pub struct Incident {
    pub id: String,
    pub key: String,
    pub code: String,
    pub status: Status,
    pub category: Category,
    pub severity: Severity,
    pub retry: Retry,
    pub exposure: Exposure,
    pub scope: Scope,
    pub first: Report,
    pub latest: Report,
    pub occurrences: i64,
    pub revision: i64,
    pub resolution: Option<Resolution>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = error::Fault)]
pub struct Fault {
    pub id: String,
    pub incident: Option<String>,
    pub code: String,
    pub message: String,
    pub category: Category,
    pub severity: Severity,
    pub retry: Retry,
    pub exposure: Exposure,
    pub source: Source,
    pub scope: Option<Scope>,
    pub context: Value,
}

impl std::fmt::Display for Fault {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "{}: {}", self.code, self.message)?;
        if let Some(incident) = self.incident.as_deref() {
            write!(out, " (incident {incident})")?;
        }
        Ok(())
    }
}

impl std::error::Error for Fault {}

#[derive(Debug, Clone)]
pub struct Draft {
    pub key: String,
    pub descriptor: Descriptor,
    pub scope: Scope,
    pub source: Source,
    pub message: String,
    pub context: Value,
}

#[derive(Debug, Clone)]
pub struct Signal {
    pub descriptor: Descriptor,
    pub source: Source,
    pub scope: Option<Scope>,
    pub message: String,
    pub context: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[schema(as = error::Kind)]
pub enum Kind {
    Opened,
    Resolved,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = error::Transition)]
pub struct Transition {
    pub id: String,
    pub incident: String,
    pub revision: i64,
    pub kind: Kind,
    pub held: Incident,
    pub occurred: String,
}

#[derive(Debug, Clone)]
pub struct Mutation {
    pub incident: Incident,
    pub error: Fault,
    pub transition: Option<Transition>,
}

pub trait Outbox {
    fn pending(&self, limit: usize) -> Result<Vec<Transition>, String>;
    fn delivered(&self, transition: &str) -> Result<(), String>;
}

pub trait Sink {
    fn publish(&self, transition: &Transition) -> Result<(), String>;
}

mod engine;
pub use engine::*;
