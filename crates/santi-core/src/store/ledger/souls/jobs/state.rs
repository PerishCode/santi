use rusqlite::params;

use super::{Record, rows};
use crate::store::Store;
use crate::{job, now, stamped};
use std::time::{Duration, UNIX_EPOCH};

impl Store {
    pub(crate) fn accept(&self, id: &str) -> Result<Record, String> {
        let conn = self.conn.lock().unwrap();
        let timestamp = now();
        conn.execute(
            r#"
            UPDATE jobs
            SET state = CASE WHEN state = 'submitting' THEN 'accepted' ELSE state END,
                accepted_at = CASE
                    WHEN state = 'submitting' THEN COALESCE(accepted_at, ?2)
                    ELSE accepted_at
                END,
                updated_at = CASE WHEN state = 'submitting' THEN ?2 ELSE updated_at END
            WHERE id = ?1
            "#,
            params![id, timestamp],
        )
        .map_err(|error| error.to_string())?;
        rows::record(&conn, id)?.ok_or_else(|| "job not found".to_string())
    }

    pub(crate) fn record(&self, id: &str) -> Result<Option<Record>, String> {
        let conn = self.conn.lock().unwrap();
        rows::record(&conn, id)
    }

    pub(crate) fn records(&self) -> Result<Vec<Record>, String> {
        self.gathered(None)
    }

    pub(crate) fn active(&self) -> Result<Vec<Record>, String> {
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            r#"
            SELECT {} FROM jobs
            WHERE state NOT IN (
                'succeeded', 'failed', 'timed_out', 'cancelled', 'unknown'
            )
            ORDER BY updated_at ASC, id ASC
            "#,
            rows::COLUMNS
        );
        let mut stmt = conn.prepare(&sql).map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map([], rows::decode)
            .map_err(|error| error.to_string())?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(|error| error.to_string())?);
        }
        Ok(records)
    }

    pub(crate) fn owned(&self, soul: &str) -> Result<Vec<Record>, String> {
        self.gathered(Some(soul))
    }

    fn gathered(&self, soul: Option<&str>) -> Result<Vec<Record>, String> {
        let conn = self.conn.lock().unwrap();
        let sql = match soul {
            Some(_) => format!(
                "SELECT {} FROM jobs WHERE soul_id = ?1 ORDER BY created_at DESC, id DESC",
                rows::COLUMNS
            ),
            None => format!(
                "SELECT {} FROM jobs ORDER BY created_at DESC, id DESC",
                rows::COLUMNS
            ),
        };
        let mut stmt = conn.prepare(&sql).map_err(|error| error.to_string())?;
        let mut records = Vec::new();
        match soul {
            Some(soul) => {
                let rows = stmt
                    .query_map([soul], rows::decode)
                    .map_err(|error| error.to_string())?;
                for row in rows {
                    records.push(row.map_err(|error| error.to_string())?);
                }
            }
            None => {
                let rows = stmt
                    .query_map([], rows::decode)
                    .map_err(|error| error.to_string())?;
                for row in rows {
                    records.push(row.map_err(|error| error.to_string())?);
                }
            }
        }
        Ok(records)
    }

    pub(crate) fn transition(
        &self,
        id: &str,
        state: job::State,
        reason: Option<&str>,
        exit: Option<i32>,
    ) -> Result<Record, String> {
        let conn = self.conn.lock().unwrap();
        let timestamp = now();
        let current = rows::record(&conn, id)?.ok_or_else(|| "job not found".to_string())?;
        let running = (state == job::State::Running).then_some(timestamp.as_str());
        let started = (state == job::State::Running)
            .then(super::epoch)
            .transpose()?;
        let next = match (state == job::State::Running, current.job.remind) {
            (true, Some(seconds)) => started.map(|millis| future(millis, seconds)).transpose()?,
            _ => None,
        };
        let finished = state.terminal().then_some(timestamp.as_str());
        conn.execute(
            r#"
            UPDATE jobs
            SET state = ?2,
                reason = ?3,
                exit_code = ?4,
                updated_at = ?5,
                started_at = COALESCE(started_at, ?6),
                started_millis = COALESCE(started_millis, ?7),
                finished_at = COALESCE(finished_at, ?8),
                next_reminder_at = CASE
                    WHEN ?9 = 1 THEN NULL
                    ELSE COALESCE(next_reminder_at, ?10)
                END
            WHERE id = ?1
            "#,
            params![
                id,
                state.encode(),
                reason,
                exit,
                timestamp,
                running,
                started,
                finished,
                state.terminal() as i64,
                next
            ],
        )
        .map_err(|error| error.to_string())?;
        rows::record(&conn, id)?.ok_or_else(|| "job not found".to_string())
    }

    pub(crate) fn acknowledge(&self, id: &str) -> Result<Record, String> {
        let conn = self.conn.lock().unwrap();
        let timestamp = now();
        conn.execute(
            r#"
            UPDATE jobs
            SET acknowledged_at = COALESCE(acknowledged_at, ?2),
                updated_at = CASE
                    WHEN acknowledged_at IS NULL THEN ?2
                    ELSE updated_at
                END
            WHERE id = ?1
            "#,
            params![id, timestamp],
        )
        .map_err(|error| error.to_string())?;
        rows::record(&conn, id)?.ok_or_else(|| "job not found".to_string())
    }

    pub fn job(&self, soul: &str, id: &str) -> Result<Option<job::Job>, String> {
        Ok(self
            .record(id)?
            .filter(|record| record.job.origin.soul == soul)
            .map(|record| record.job))
    }

    pub fn jobs(&self, soul: &str) -> Result<Vec<job::Job>, String> {
        Ok(self
            .owned(soul)?
            .into_iter()
            .map(|record| record.job)
            .collect())
    }
}

fn future(millis: i64, seconds: u64) -> Result<String, String> {
    let base = u64::try_from(millis).map_err(|_| "job start time is out of range".to_string())?;
    let delta = seconds
        .checked_mul(1000)
        .ok_or_else(|| "job reminder interval is out of range".to_string())?;
    let total = base
        .checked_add(delta)
        .ok_or_else(|| "job reminder time is out of range".to_string())?;
    stamped(UNIX_EPOCH + Duration::from_millis(total))
}
