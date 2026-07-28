use santi_model::job;

#[derive(Clone, Copy)]
pub struct CapabilityDraft<'a> {
    pub digest: &'a str,
    pub expires: i64,
    pub soul: &'a str,
    pub strand: &'a str,
    pub turn: &'a str,
    pub call: &'a str,
    pub effect: &'a str,
    pub created: &'a str,
}

#[derive(Clone, Copy)]
pub struct JobDraft<'a> {
    pub tag: &'a str,
    pub description: &'a str,
    pub command: &'a str,
    pub cwd: Option<&'a str>,
    pub timeout_seconds: u64,
    pub output_limit_bytes: u64,
    pub remind_every_seconds: Option<u64>,
    pub request_sha256: &'a str,
    pub generation: &'a str,
    pub supervisor_ref: &'a str,
    pub created: &'a str,
}

pub struct TransitionDraft<'a> {
    pub state: job::State,
    pub reason: Option<&'a str>,
    pub exit_code: Option<i32>,
    pub occurred: &'a str,
    pub started_millis: Option<i64>,
    pub next_reminder: Option<&'a str>,
}

#[derive(Clone, Copy)]
pub struct AttentionDraft<'a> {
    pub job: &'a str,
    pub base: u64,
    pub at: &'a str,
    pub runtime: bool,
    pub output: bool,
    pub reminded: bool,
    pub tick: u64,
    pub next: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct JobRecord {
    pub job: job::Job,
    pub generation: String,
    pub supervisor: String,
    pub started_millis: Option<i64>,
    pub attention_revision: u64,
    pub runtime_warned: bool,
    pub output_warned: bool,
    pub reminder_tick: u64,
}

#[derive(Debug, Clone)]
pub enum Prepared {
    New(JobRecord),
    Existing(JobRecord),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpiredJob {
    pub id: String,
    pub key: String,
}
