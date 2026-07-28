use serde::Serialize;

use super::Service;
use crate::job;
use crate::store::{JobEntry, JobGrant, JobPrepared, JobRecord};

mod attention;
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

#[derive(Debug, Clone)]
pub struct Terminal {
    pub state: job::State,
    pub reason: Option<String>,
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
    fn detach(&self, _launch: &Launch) -> Result<(), String> {
        Err("job supervisor is unavailable".to_string())
    }

    fn observe(&self, _launch: &Launch) -> Result<Observation, String> {
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
            self.detach(&record)?;
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
        Ok(self
            .store
            .owned(soul)?
            .into_iter()
            .map(|record| {
                let fallback = record.clone();
                self.refresh(record)
                    .map(|record| record.job)
                    .unwrap_or_else(|error| {
                        eprintln!(
                            "santi: job observation failed job={} detail={error}",
                            fallback.job.id
                        );
                        fallback.job
                    })
            })
            .collect())
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

    fn launch(&self, record: &JobRecord) -> Result<Launch, String> {
        let cwd = self.situated(
            &record.job.origin.strand,
            &record.job.origin.soul,
            record.job.cwd.as_deref(),
        )?;
        std::fs::create_dir_all(&cwd).map_err(|error| error.to_string())?;
        Ok(Launch {
            job: record.job.clone(),
            stamp: record.stamp.clone(),
            sidecar: record.sidecar.clone(),
            cwd: cwd.display().to_string(),
            directory: self.jobhome(record).display().to_string(),
        })
    }

    fn refresh(&self, record: JobRecord) -> Result<JobRecord, String> {
        if record.job.acknowledged.is_some() {
            return Ok(record);
        }
        let observation = self.supervisor.observe(&self.launch(&record)?)?;
        let claimed = matches!(
            &observation,
            Observation::Claimed | Observation::Running | Observation::Terminal(_)
        );
        let record = if claimed && record.job.state == job::State::Submitting {
            self.store.accept(&record.job.id)?
        } else {
            record
        };
        let transition = match observation {
            Observation::Claimed => None,
            Observation::Running => Some((job::State::Running, None, None)),
            Observation::Terminal(terminal) => {
                Some((terminal.state, terminal.reason, terminal.exit))
            }
            Observation::Aborted => Some((
                job::State::Failed,
                Some("submission_aborted".to_string()),
                None,
            )),
            Observation::Missing if record.job.state.terminal() => None,
            Observation::Missing
                if record.job.state == job::State::Submitting
                    && self.handoffs.lock().unwrap().contains(&record.stamp) =>
            {
                None
            }
            Observation::Missing if record.job.state == job::State::Submitting => Some((
                job::State::Failed,
                Some("submission_aborted".to_string()),
                None,
            )),
            Observation::Missing => Some((
                job::State::Unknown,
                Some("sidecar_evidence_missing".to_string()),
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

    fn detach(&self, record: &JobRecord) -> Result<(), String> {
        self.handoffs.lock().unwrap().insert(record.stamp.clone());
        let result = self
            .launch(record)
            .and_then(|launch| self.supervisor.detach(&launch));
        self.handoffs.lock().unwrap().remove(&record.stamp);
        result
    }
}
