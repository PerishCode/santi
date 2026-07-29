use super::{ExpiredJob, JobRecord, Store, read};
use keel::{Op, Rank, form};
use santi_model::job;

const ACTIVE: &[&str] = &["submitting", "accepted", "running", "cancelling"];
const TERMINAL: &[&str] = &["succeeded", "failed", "timed_out", "cancelled", "unknown"];

pub(super) struct Projection<'a>(&'a Store);

impl<'a> Projection<'a> {
    pub fn new(store: &'a Store) -> Self {
        Self(store)
    }

    pub async fn record(&self, tag: &str) -> Result<Option<JobRecord>, String> {
        let Some(row) = read::one(&self.0.core, "Job", "tag", tag).await? else {
            return Ok(None);
        };
        self.decode(&row).await.map(Some)
    }

    pub async fn active(&self) -> Result<Vec<JobRecord>, String> {
        let rows = self
            .0
            .core
            .ask(
                &form("Job")
                    .any("state", ACTIVE)
                    .order("updated", Rank::Asc)
                    .order("tag", Rank::Asc),
            )
            .await
            .map_err(read::error)?;
        self.decode_all(rows.rows()).await
    }

    pub async fn jobs(&self, soul: &str) -> Result<Vec<JobRecord>, String> {
        let rows = self
            .0
            .core
            .ask(
                &form("Job")
                    .order("created", Rank::Desc)
                    .order("tag", Rank::Desc),
            )
            .await
            .map_err(read::error)?;
        let mut records = Vec::new();
        for row in rows.rows() {
            let record = self.decode(row).await?;
            if record.job.origin.soul == soul {
                records.push(record);
            }
        }
        Ok(records)
    }

    pub async fn expired(&self, cutoff: &str, limit: usize) -> Result<Vec<ExpiredJob>, String> {
        let rows = self
            .0
            .core
            .ask(
                &form("Job")
                    .when("acknowledged", Op::Le, cutoff)
                    .any("state", TERMINAL)
                    .order("acknowledged", Rank::Asc)
                    .order("tag", Rank::Asc)
                    .top(limit),
            )
            .await
            .map_err(read::error)?;
        rows.rows()
            .iter()
            .map(|row| {
                let id = read::text(row, "tag")?.to_string();
                let generation = read::text(row, "generation")?;
                Ok(ExpiredJob {
                    key: if generation.starts_with("stamp_") {
                        generation.to_string()
                    } else {
                        id.clone()
                    },
                    id,
                })
            })
            .collect()
    }

    async fn decode(&self, row: &keel::Row) -> Result<JobRecord, String> {
        let strand = self.related("Strand", read::int(row, "strand")?).await?;
        let soul = read::related(&self.0.core, "Soul", read::int(&strand, "soul")?).await?;
        let strand_tag = read::text(&strand, "tag")?.to_string();
        Ok(JobRecord {
            job: job::Job {
                id: read::text(row, "tag")?.to_string(),
                origin: job::Origin {
                    soul,
                    strand: strand_tag,
                    turn: read::related(&self.0.core, "Turn", read::int(row, "turn")?).await?,
                    call: read::related(&self.0.core, "ToolCall", read::int(row, "call")?).await?,
                    effect: read::related(&self.0.core, "StrandEffect", read::int(row, "effect")?)
                        .await?,
                },
                description: read::text(row, "description")?.to_string(),
                command: read::text(row, "command")?.to_string(),
                cwd: row.text("cwd").map(str::to_string),
                timeout_seconds: unsigned(row, "timeout_seconds")?,
                output_limit_bytes: unsigned(row, "output_limit_bytes")?,
                remind: optional_unsigned(row, "remind_every_seconds")?,
                state: state(read::text(row, "state")?)?,
                reason: row.text("reason").map(str::to_string),
                exit_code: row
                    .int("exit_code")
                    .map(i32::try_from)
                    .transpose()
                    .map_err(|error| error.to_string())?,
                created: read::text(row, "created")?.to_string(),
                updated: read::text(row, "updated")?.to_string(),
                accepted: row.text("accepted").map(str::to_string),
                started: row.text("started").map(str::to_string),
                last: row.text("last_reminded").map(str::to_string),
                next: row.text("next_reminder").map(str::to_string),
                finished: row.text("finished").map(str::to_string),
                acknowledged: row.text("acknowledged").map(str::to_string),
            },
            generation: read::text(row, "generation")?.to_string(),
            supervisor: read::text(row, "supervisor_ref")?.to_string(),
            started_millis: row.int("started_millis"),
            attention_revision: unsigned(row, "attention_revision")?,
            runtime_warned: row.text("runtime_warned").is_some(),
            output_warned: row.text("output_warned").is_some(),
            reminder_tick: unsigned(row, "reminder_tick")?,
        })
    }

    async fn decode_all(&self, rows: &[keel::Row]) -> Result<Vec<JobRecord>, String> {
        let mut records = Vec::with_capacity(rows.len());
        for row in rows {
            records.push(self.decode(row).await?);
        }
        Ok(records)
    }

    async fn related(&self, unit: &str, key: i64) -> Result<keel::Row, String> {
        read::one(&self.0.core, unit, "id", &key.to_string())
            .await?
            .ok_or_else(|| format!("{unit} {key} missing"))
    }
}

fn unsigned(row: &keel::Row, field: &str) -> Result<u64, String> {
    u64::try_from(read::int(row, field)?).map_err(|error| error.to_string())
}

fn optional_unsigned(row: &keel::Row, field: &str) -> Result<Option<u64>, String> {
    row.int(field)
        .map(u64::try_from)
        .transpose()
        .map_err(|error| error.to_string())
}

fn state(value: &str) -> Result<job::State, String> {
    match value {
        "submitting" => Ok(job::State::Submitting),
        "accepted" => Ok(job::State::Accepted),
        "running" => Ok(job::State::Running),
        "cancelling" => Ok(job::State::Cancelling),
        "succeeded" => Ok(job::State::Succeeded),
        "failed" => Ok(job::State::Failed),
        "timed_out" => Ok(job::State::TimedOut),
        "cancelled" => Ok(job::State::Cancelled),
        "unknown" => Ok(job::State::Unknown),
        value => Err(format!("unknown job state {value}")),
    }
}
