use serde::{Deserialize, Serialize};

use super::Service;
use crate::job;

mod attention;
mod control;
mod draft;
mod logs;
mod paths;
mod retention;
mod watch;

pub use logs::Read;

#[derive(Debug, Clone, Serialize)]
pub struct Draft {
    pub description: String,
    pub command: String,
    pub cwd: Option<String>,
    pub timeout: Option<u64>,
    pub output: Option<u64>,
    pub remind: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Launch {
    pub job: job::Job,
    pub stamp: String,
    pub sidecar: String,
    pub cwd: String,
    pub directory: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Terminal {
    pub state: job::State,
    pub reason: Option<String>,
    #[serde(rename = "exit_code")]
    pub exit: Option<i32>,
}

#[derive(Debug, Clone)]
pub enum Observation {
    Claimed,
    Running,
    Terminal(Terminal),
    Aborted,
    Missing,
}

pub trait Supervisor: Send + Sync {
    fn detach(&self, launch: &Launch) -> Result<(), String>;
    fn observe(&self, launch: &Launch) -> Result<Observation, String>;
    fn stop(&self, launch: &Launch) -> Result<(), String>;
    fn acknowledge(&self, launch: &Launch) -> Result<(), String>;
}

pub(super) struct Unavailable;

impl Supervisor for Unavailable {
    fn detach(&self, _: &Launch) -> Result<(), String> {
        Err("job supervisor is unavailable".to_string())
    }

    fn observe(&self, _: &Launch) -> Result<Observation, String> {
        Err("job supervisor is unavailable".to_string())
    }

    fn stop(&self, _: &Launch) -> Result<(), String> {
        Err("job supervisor is unavailable".to_string())
    }

    fn acknowledge(&self, _: &Launch) -> Result<(), String> {
        Err("job supervisor is unavailable".to_string())
    }
}
