use super::{Store, read};
use keel::{Op, form};
use santi_model::job;

mod attention;
mod projection;
mod types;
mod write;

pub use types::{
    AttentionDraft, CapabilityDraft, ExpiredJob, JobDraft, JobRecord, Prepared, TransitionDraft,
};

impl Store {
    pub async fn create_capability(&self, draft: CapabilityDraft<'_>) -> Result<(), String> {
        self.core
            .batch(async |tx| write::capability(tx, draft).await)
            .await
            .map_err(read::error)
    }

    pub async fn expire_capabilities(&self, now_millis: i64) -> Result<usize, String> {
        self.core
            .batch(async |tx| {
                let rows = tx
                    .ask(
                        &form("JobCapability")
                            .when("expires", Op::Lt, &now_millis.to_string())
                            .missing("consumed"),
                    )
                    .await?;
                for row in rows.rows() {
                    tx.end("JobCapability", row.key()).await?;
                }
                Ok(rows.rows().len())
            })
            .await
            .map_err(read::error)
    }

    pub async fn prepare_job(
        &self,
        capability_sha256: &str,
        draft: JobDraft<'_>,
        now_millis: i64,
    ) -> Result<Prepared, String> {
        let (created, tag) = self
            .core
            .batch(async |tx| write::prepare(tx, capability_sha256, draft, now_millis).await)
            .await
            .map_err(read::error)?;
        let record = projection::Projection::new(self)
            .record(&tag)
            .await?
            .ok_or_else(|| "prepared job missing".to_string())?;
        if created {
            Ok(Prepared::New(record))
        } else {
            Ok(Prepared::Existing(record))
        }
    }

    pub async fn accept_job(&self, tag: &str, occurred: &str) -> Result<JobRecord, String> {
        self.core
            .batch(async |tx| write::accept(tx, tag, occurred).await)
            .await
            .map_err(read::error)?;
        self.need_job(tag).await
    }

    pub async fn transition_job(
        &self,
        tag: &str,
        draft: TransitionDraft<'_>,
    ) -> Result<JobRecord, String> {
        self.core
            .batch(async |tx| write::Writer::new(tx).transition(tag, draft).await)
            .await
            .map_err(read::error)?;
        self.need_job(tag).await
    }

    pub async fn acknowledge_job(&self, tag: &str, occurred: &str) -> Result<JobRecord, String> {
        self.core
            .batch(async |tx| write::Writer::new(tx).acknowledge(tag, occurred).await)
            .await
            .map_err(read::error)?;
        self.need_job(tag).await
    }

    pub async fn job(&self, soul: &str, tag: &str) -> Result<Option<job::Job>, String> {
        Ok(projection::Projection::new(self)
            .record(tag)
            .await?
            .filter(|record| record.job.origin.soul == soul)
            .map(|record| record.job))
    }

    pub async fn job_record(&self, tag: &str) -> Result<Option<JobRecord>, String> {
        projection::Projection::new(self).record(tag).await
    }

    pub async fn jobs(&self, soul: &str) -> Result<Vec<job::Job>, String> {
        Ok(projection::Projection::new(self)
            .jobs(soul)
            .await?
            .into_iter()
            .map(|record| record.job)
            .collect())
    }

    pub async fn active_jobs(&self) -> Result<Vec<JobRecord>, String> {
        projection::Projection::new(self).active().await
    }

    pub async fn expired_jobs(
        &self,
        cutoff: &str,
        limit: usize,
    ) -> Result<Vec<ExpiredJob>, String> {
        projection::Projection::new(self)
            .expired(cutoff, limit)
            .await
    }

    pub async fn retained_job(&self, key: &str) -> Result<bool, String> {
        if read::one(&self.core, "Job", "tag", key).await?.is_some() {
            return Ok(true);
        }
        Ok(read::one(&self.core, "Job", "generation", key)
            .await?
            .is_some())
    }

    pub async fn purge_job(&self, tag: &str, cutoff: &str) -> Result<bool, String> {
        self.core
            .batch(async |tx| {
                let Some(job) = tx.one(&form("Job").when("tag", Op::Eq, tag)).await? else {
                    return Ok(false);
                };
                if !terminal(job.text("state"))
                    || job
                        .text("acknowledged")
                        .is_none_or(|acknowledged| acknowledged > cutoff)
                {
                    return Ok(false);
                }
                let capabilities = tx
                    .ask(&form("JobCapability").when("consumed", Op::Eq, &job.key().to_string()))
                    .await?;
                for capability in capabilities.rows() {
                    tx.end("JobCapability", capability.key()).await?;
                }
                tx.end("Job", job.key()).await?;
                Ok(true)
            })
            .await
            .map_err(read::error)
    }

    async fn need_job(&self, tag: &str) -> Result<JobRecord, String> {
        projection::Projection::new(self)
            .record(tag)
            .await?
            .ok_or_else(|| "job not found".to_string())
    }
}

fn terminal(state: Option<&str>) -> bool {
    matches!(
        state,
        Some("succeeded" | "failed" | "timed_out" | "cancelled" | "unknown")
    )
}
