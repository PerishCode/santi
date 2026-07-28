use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::store::Store;
use crate::{job, now, tag};

mod attention;
mod retention;
mod rows;
mod state;

pub(crate) use retention::Expired;

const TTL: u64 = 120;

pub(crate) struct Grant<'a> {
    pub soul: &'a str,
    pub strand: &'a str,
    pub turn: &'a str,
    pub call: &'a str,
    pub effect: &'a str,
}

pub(crate) struct Entry {
    pub description: String,
    pub command: String,
    pub cwd: Option<String>,
    pub timeout: u64,
    pub output: u64,
    pub remind: Option<u64>,
    pub digest: String,
}

#[derive(Debug, Clone)]
pub(crate) struct Record {
    pub job: job::Job,
    pub stamp: String,
    pub sidecar: String,
    pub started: Option<i64>,
    pub revision: u64,
    pub runtime: bool,
    pub output: bool,
    pub reminder: u64,
}

pub(crate) enum Prepared {
    New(Record),
    Existing(Record),
}

pub(crate) struct Attention<'a> {
    pub id: &'a str,
    pub base: u64,
    pub at: &'a str,
    pub runtime: bool,
    pub output: bool,
    pub reminded: bool,
    pub tick: u64,
    pub next: Option<&'a str>,
}

impl Store {
    pub(crate) fn grant(&self, origin: Grant<'_>) -> Result<String, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let keeper = tx
            .query_row(
                "SELECT soul_id FROM strands WHERE id = ?1",
                [origin.strand],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "strand not found".to_string())?;
        if keeper != origin.soul {
            return Err("job capability origin does not match strand owner".to_string());
        }

        let current = epoch()?;
        tx.execute(
            "DELETE FROM job_capabilities WHERE expires_at < ?1 AND consumed_job_id IS NULL",
            [current],
        )
        .map_err(|error| error.to_string())?;
        let token = tag("jobcap");
        let digest = digest(&token);
        let expires = current.saturating_add((TTL * 1000) as i64);
        tx.execute(
            r#"
            INSERT INTO job_capabilities (
                digest, soul_id, strand_id, turn_id, tool_call_id, effect_id,
                expires_at, consumed_job_id, request_sha256, created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, ?8)
            "#,
            params![
                digest,
                origin.soul,
                origin.strand,
                origin.turn,
                origin.call,
                origin.effect,
                expires,
                now()
            ],
        )
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(token)
    }

    pub(crate) fn prepare(&self, capability: &str, draft: Entry) -> Result<Prepared, String> {
        let proof = digest(capability);
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let held = tx
            .query_row(
                r#"
                SELECT soul_id, strand_id, turn_id, tool_call_id, effect_id,
                       expires_at, consumed_job_id, request_sha256
                FROM job_capabilities WHERE digest = ?1
                "#,
                [&proof],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "invalid job capability".to_string())?;
        let (soul, strand, turn, call, effect, expires, consumed, accepted) = held;
        if let Some(job) = consumed {
            if accepted.as_deref() != Some(draft.digest.as_str()) {
                return Err("job capability conflicts with its accepted request".to_string());
            }
            let record = rows::record(&tx, &job)?
                .ok_or_else(|| "consumed job capability has no job record".to_string())?;
            tx.commit().map_err(|error| error.to_string())?;
            return Ok(Prepared::Existing(record));
        }
        if epoch()? > expires {
            return Err("job capability expired".to_string());
        }

        let id = tag("job");
        let stamp = tag("stamp");
        let sidecar = format!("santi-{}.service", stamp.replace('_', "-"));
        let timestamp = now();
        let timeout =
            i64::try_from(draft.timeout).map_err(|_| "job timeout is out of range".to_string())?;
        let output = i64::try_from(draft.output)
            .map_err(|_| "job output limit is out of range".to_string())?;
        let remind = draft
            .remind
            .map(i64::try_from)
            .transpose()
            .map_err(|_| "job reminder interval is out of range".to_string())?;
        tx.execute(
            r#"
            INSERT INTO jobs (
                id, soul_id, strand_id, turn_id, tool_call_id, effect_id,
                description, command, cwd, timeout_seconds, output_limit_bytes,
                remind_every_seconds,
                request_sha256, capability_sha256, generation, supervisor_ref,
                state, reason, exit_code, created_at, updated_at, accepted_at,
                started_at, started_millis, finished_at, acknowledged_at,
                attention_revision, runtime_warned_at, output_warned_at,
                last_reminded_at, next_reminder_at, reminder_tick
            )
            VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                ?12, ?13, ?14, ?15, ?16, 'submitting', NULL, NULL,
                ?17, ?17, NULL, NULL, NULL, NULL, NULL, 0, NULL, NULL, NULL, NULL, 0
            )
            "#,
            params![
                id,
                soul,
                strand,
                turn,
                call,
                effect,
                draft.description,
                draft.command,
                draft.cwd,
                timeout,
                output,
                remind,
                draft.digest,
                proof,
                stamp,
                sidecar,
                timestamp
            ],
        )
        .map_err(|error| error.to_string())?;
        tx.execute(
            r#"
            UPDATE job_capabilities
            SET consumed_job_id = ?2, request_sha256 = ?3
            WHERE digest = ?1 AND consumed_job_id IS NULL
            "#,
            params![proof, id, draft.digest],
        )
        .map_err(|error| error.to_string())?;
        let record =
            rows::record(&tx, &id)?.ok_or_else(|| "created job record missing".to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(Prepared::New(record))
    }
}

pub(super) fn epoch() -> Result<i64, String> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    i64::try_from(elapsed.as_millis()).map_err(|_| "system clock is out of range".to_string())
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
