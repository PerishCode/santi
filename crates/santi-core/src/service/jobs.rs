use std::path::PathBuf;

use serde::Serialize;

use super::Service;
use crate::job;
use crate::store::{JobEntry, JobGrant, JobPrepared, JobRecord};

mod attention;
mod draft;
mod logs;

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
    pub generation: String,
    pub supervisor: String,
    pub cwd: String,
    pub directory: String,
}

#[derive(Debug, Clone)]
pub struct Terminal {
    pub state: job::State,
    pub reason: Option<String>,
    pub exit: Option<i32>,
}

#[derive(Debug, Clone)]
pub enum Observation {
    Accepted,
    Running,
    Terminal(Terminal),
    Missing,
}

pub trait Supervisor: Send + Sync {
    fn ensure(&self, launch: &Launch) -> Result<(), String>;
    fn inspect(&self, launch: &Launch) -> Result<Observation, String>;
    fn stop(&self, launch: &Launch) -> Result<(), String>;
    fn acknowledge(&self, launch: &Launch) -> Result<(), String>;
}

pub(super) struct Unavailable;

impl Supervisor for Unavailable {
    fn ensure(&self, _launch: &Launch) -> Result<(), String> {
        Err("job supervisor is unavailable".to_string())
    }

    fn inspect(&self, _launch: &Launch) -> Result<Observation, String> {
        Err("job supervisor is unavailable".to_string())
    }

    fn stop(&self, _launch: &Launch) -> Result<(), String> {
        Err("job supervisor is unavailable".to_string())
    }

    fn acknowledge(&self, _launch: &Launch) -> Result<(), String> {
        Err("job supervisor is unavailable".to_string())
    }
}

impl Service {
    pub(crate) fn permit(
        &self,
        strand: &str,
        turn: &str,
        call: &str,
        effect: &str,
    ) -> Result<String, String> {
        let soul = self.store.keeper(strand)?;
        self.store.grant(JobGrant {
            soul: &soul,
            strand,
            turn,
            call,
            effect,
        })
    }

    pub fn spawn(&self, capability: &str, draft: Draft) -> Result<job::Accepted, String> {
        let normalized = draft::normalize(draft)?;
        let prepared = self.store.prepare(
            capability,
            JobEntry {
                description: normalized.description,
                command: normalized.command,
                cwd: normalized.cwd,
                timeout: normalized.timeout,
                output: normalized.output,
                remind: normalized.remind,
                digest: normalized.digest,
            },
        )?;
        let record = match prepared {
            JobPrepared::New(record) | JobPrepared::Existing(record) => record,
        };
        if record.job.state == job::State::Submitting {
            self.supervisor.ensure(&self.launch(&record)?)?;
        }
        let record = if record.job.state == job::State::Submitting {
            self.store.accept(&record.job.id)?
        } else {
            record
        };
        Ok(job::Accepted { job: record.job })
    }

    pub fn job(&self, soul: &str, id: &str) -> Result<Option<job::Job>, String> {
        let Some(record) = self
            .store
            .record(id)?
            .filter(|record| record.job.origin.soul == soul)
        else {
            return Ok(None);
        };
        self.refresh(record).map(|record| Some(record.job))
    }

    pub fn jobs(&self, soul: &str) -> Result<Vec<job::Job>, String> {
        self.store
            .owned(soul)?
            .into_iter()
            .map(|record| self.refresh(record).map(|record| record.job))
            .collect()
    }

    pub fn cancel(&self, soul: &str, id: &str) -> Result<Option<job::Job>, String> {
        let Some(record) = self
            .store
            .record(id)?
            .filter(|record| record.job.origin.soul == soul)
        else {
            return Ok(None);
        };
        let record = self.refresh(record)?;
        if record.job.state.terminal() {
            return Ok(Some(record.job));
        }
        let record =
            self.store
                .transition(id, job::State::Cancelling, Some("cancel_requested"), None)?;
        self.supervisor.stop(&self.launch(&record)?)?;
        self.refresh(record).map(|record| Some(record.job))
    }

    pub fn ack(&self, soul: &str, id: &str) -> Result<Option<job::Job>, String> {
        let Some(record) = self
            .store
            .record(id)?
            .filter(|record| record.job.origin.soul == soul)
        else {
            return Ok(None);
        };
        let record = self.refresh(record)?;
        if !record.job.state.terminal() {
            return Err("only a terminal job can be acknowledged".to_string());
        }
        self.supervisor.acknowledge(&self.launch(&record)?)?;
        self.store.acknowledge(id).map(|record| Some(record.job))
    }

    pub(crate) fn recover(&self) -> Result<(), String> {
        for record in self.store.records()? {
            self.refresh(record)?;
        }
        Ok(())
    }

    fn sweep(&self) -> Result<(), String> {
        for record in self.store.active()? {
            let id = record.job.id.clone();
            let result = self
                .refresh(record)
                .and_then(|record| attention::capture(self, record));
            if let Err(error) = result {
                eprintln!("santi: job attention failed job={id} detail={error}");
            }
        }
        Ok(())
    }

    pub async fn watch(&self) {
        while !self.closing() {
            if let Err(error) = self.sweep() {
                eprintln!("santi: job attention scan failed: {error}");
            }
            self.rouse();
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }

    fn launch(&self, record: &JobRecord) -> Result<Launch, String> {
        let cwd = self.situated(
            &record.job.origin.strand,
            &record.job.origin.soul,
            record.job.cwd.as_deref(),
        )?;
        std::fs::create_dir_all(&cwd).map_err(|error| error.to_string())?;
        Ok(Launch {
            job: record.job.clone(),
            generation: record.generation.clone(),
            supervisor: record.supervisor.clone(),
            cwd: cwd.display().to_string(),
            directory: self.jobhome(&record.job.id).display().to_string(),
        })
    }

    fn refresh(&self, record: JobRecord) -> Result<JobRecord, String> {
        if record.job.acknowledged.is_some() {
            return Ok(record);
        }
        let observation = self.supervisor.inspect(&self.launch(&record)?)?;
        let transition = match observation {
            Observation::Accepted if record.job.state == job::State::Submitting => {
                return self.store.accept(&record.job.id);
            }
            Observation::Accepted => None,
            Observation::Running => Some((job::State::Running, None, None)),
            Observation::Terminal(terminal) => {
                Some((terminal.state, terminal.reason, terminal.exit))
            }
            Observation::Missing if record.job.state.terminal() => None,
            Observation::Missing => Some((
                job::State::Unknown,
                Some("supervisor_evidence_missing".to_string()),
                None,
            )),
        };
        let Some((state, reason, exit)) = transition else {
            return Ok(record);
        };
        let incomplete = state == job::State::Running && record.started.is_none();
        if record.job.state == state
            && record.job.reason == reason
            && record.job.exit_code == exit
            && !incomplete
        {
            return Ok(record);
        }
        self.store
            .transition(&record.job.id, state, reason.as_deref(), exit)
    }

    fn jobhome(&self, id: &str) -> PathBuf {
        self.runtime().join("jobs").join(id)
    }
}
