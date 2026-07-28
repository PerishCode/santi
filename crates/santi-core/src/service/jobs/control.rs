use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;
use santi_estate::{JobRecord, Prepared};

impl Service {
    pub(crate) async fn permit(
        &self,
        strand: &str,
        turn: &str,
        call: &str,
        effect: &str,
    ) -> Result<String, String> {
        let soul = self
            .store
            .strand(strand)
            .await?
            .map(|strand| strand.soul)
            .ok_or_else(|| "strand not found".to_string())?;
        let capability = crate::tag("jobcap");
        let digest = hex::encode(Sha256::digest(capability.as_bytes()));
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_millis();
        let expires = i64::try_from(now.saturating_add(120_000))
            .map_err(|_| "job capability expiry is out of range".to_string())?;
        self.store
            .create_capability(santi_estate::CapabilityDraft {
                digest: &digest,
                expires,
                soul: &soul,
                strand,
                turn,
                call,
                effect,
                created: &crate::now(),
            })
            .await?;
        Ok(capability)
    }

    pub async fn spawn(&self, capability: &str, draft: Draft) -> Result<job::Accepted, String> {
        let normalized = draft::normalize(draft)?;
        let capability = hex::encode(Sha256::digest(capability.as_bytes()));
        let tag = crate::tag("job");
        let generation = crate::tag("stamp");
        let supervisor = crate::tag("sidecar");
        let now = crate::now();
        let prepared = self
            .store
            .prepare_job(
                &capability,
                santi_estate::JobDraft {
                    tag: &tag,
                    description: &normalized.description,
                    command: &normalized.command,
                    cwd: normalized.cwd.as_deref(),
                    timeout_seconds: normalized.timeout,
                    output_limit_bytes: normalized.output,
                    remind_every_seconds: normalized.remind,
                    request_sha256: &normalized.digest,
                    generation: &generation,
                    supervisor_ref: &supervisor,
                    created: &now,
                },
                epoch()?,
            )
            .await?;
        let record = match prepared {
            Prepared::New(record) | Prepared::Existing(record) => record,
        };
        if record.job.state == job::State::Submitting {
            self.detach(&record)?;
        }
        let record = if record.job.state == job::State::Submitting {
            self.store.accept_job(&record.job.id, &crate::now()).await?
        } else {
            record
        };
        Ok(job::Accepted { job: record.job })
    }

    pub async fn job(&self, soul: &str, id: &str) -> Result<Option<job::Job>, String> {
        let Some(record) = self.store.job_record(id).await? else {
            return Ok(None);
        };
        if record.job.origin.soul != soul {
            return Ok(None);
        }
        if record.job.state.terminal() || record.job.acknowledged.is_some() {
            return Ok(Some(record.job));
        }
        self.refresh(record).await.map(|record| Some(record.job))
    }

    pub async fn jobs(&self, soul: &str) -> Result<Vec<job::Job>, String> {
        let active = self.store.active_jobs().await?;
        for record in active
            .into_iter()
            .filter(|record| record.job.origin.soul == soul)
        {
            if let Err(error) = self.refresh(record.clone()).await {
                eprintln!(
                    "santi: job observation failed job={} detail={error}",
                    record.job.id
                );
            }
        }
        self.store.jobs(soul).await
    }

    pub async fn cancel(&self, soul: &str, id: &str) -> Result<Option<job::Job>, String> {
        let Some(record) = self.store.job_record(id).await? else {
            return Ok(None);
        };
        if record.job.origin.soul != soul {
            return Ok(None);
        }
        if record.job.state.terminal() {
            return Ok(Some(record.job));
        }
        let record = self.refresh(record).await?;
        if record.job.state.terminal() {
            return Ok(Some(record.job));
        }
        let record = self
            .store
            .transition_job(
                id,
                santi_estate::TransitionDraft {
                    state: job::State::Cancelling,
                    reason: Some("cancel_requested"),
                    exit_code: None,
                    occurred: &crate::now(),
                    started_millis: None,
                    next_reminder: None,
                },
            )
            .await?;
        self.supervisor.stop(&self.launch(&record)?)?;
        self.refresh(record).await.map(|record| Some(record.job))
    }

    pub async fn ack(&self, soul: &str, id: &str) -> Result<Option<job::Job>, String> {
        let Some(record) = self.store.job_record(id).await? else {
            return Ok(None);
        };
        if record.job.origin.soul != soul {
            return Ok(None);
        }
        if record.job.acknowledged.is_some() {
            return Ok(Some(record.job));
        }
        let record = if record.job.state.terminal() {
            record
        } else {
            self.refresh(record).await?
        };
        if !record.job.state.terminal() {
            return Err("only a terminal job can be acknowledged".to_string());
        }
        self.supervisor.acknowledge(&self.launch(&record)?)?;
        self.store
            .acknowledge_job(id, &crate::now())
            .await
            .map(|record| Some(record.job))
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
            stamp: record.generation.clone(),
            sidecar: record.supervisor.clone(),
            cwd: cwd.display().to_string(),
            directory: self.jobhome(record).display().to_string(),
        })
    }

    pub(super) async fn refresh(&self, record: JobRecord) -> Result<JobRecord, String> {
        if record.job.acknowledged.is_some() {
            return Ok(record);
        }
        let observation = self.supervisor.observe(&self.launch(&record)?)?;
        let claimed = matches!(
            &observation,
            Observation::Claimed | Observation::Running | Observation::Terminal(_)
        );
        let record = if claimed && record.job.state == job::State::Submitting {
            self.store.accept_job(&record.job.id, &crate::now()).await?
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
                    && self.handoffs.lock().unwrap().contains(&record.generation) =>
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
        let incomplete = state == job::State::Running && record.started_millis.is_none();
        if record.job.state == state
            && record.job.reason == reason
            && record.job.exit_code == exit
            && !incomplete
        {
            return Ok(record);
        }
        self.store
            .transition_job(
                &record.job.id,
                santi_estate::TransitionDraft {
                    state,
                    reason: reason.as_deref(),
                    exit_code: exit,
                    occurred: &crate::now(),
                    started_millis: incomplete.then(epoch).transpose()?,
                    next_reminder: None,
                },
            )
            .await
    }

    fn detach(&self, record: &JobRecord) -> Result<(), String> {
        self.handoffs
            .lock()
            .unwrap()
            .insert(record.generation.clone());
        let result = self
            .launch(record)
            .and_then(|launch| self.supervisor.detach(&launch));
        self.handoffs.lock().unwrap().remove(&record.generation);
        result
    }
}

fn epoch() -> Result<i64, String> {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_millis(),
    )
    .map_err(|_| "current time is out of range".to_string())
}
