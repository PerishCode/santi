use rusqlite::{OptionalExtension, params};

use crate::store::Store;

#[derive(Debug, Clone)]
pub(crate) struct Expired {
    pub id: String,
    pub key: String,
}

impl Store {
    pub(crate) fn expired(&self, cutoff: &str, limit: usize) -> Result<Vec<Expired>, String> {
        let limit = i64::try_from(limit).map_err(|error| error.to_string())?;
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                r#"
                SELECT id, CASE
                    WHEN generation LIKE 'stamp_%' THEN generation
                    ELSE id
                END
                FROM jobs
                WHERE acknowledged_at IS NOT NULL
                  AND acknowledged_at <= ?1
                  AND state IN (
                      'succeeded', 'failed', 'timed_out', 'cancelled', 'unknown'
                  )
                ORDER BY acknowledged_at ASC, id ASC
                LIMIT ?2
                "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![cutoff, limit], |row| {
                Ok(Expired {
                    id: row.get(0)?,
                    key: row.get(1)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub(crate) fn retained(&self, key: &str) -> Result<bool, String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT 1 FROM jobs WHERE id = ?1 OR generation = ?1 LIMIT 1",
            [key],
            |_| Ok(()),
        )
        .optional()
        .map(|value| value.is_some())
        .map_err(|error| error.to_string())
    }

    pub(crate) fn purge(&self, id: &str, cutoff: &str) -> Result<bool, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let removed = tx
            .execute(
                r#"
                DELETE FROM jobs
                WHERE id = ?1
                  AND acknowledged_at IS NOT NULL
                  AND acknowledged_at <= ?2
                  AND state IN (
                      'succeeded', 'failed', 'timed_out', 'cancelled', 'unknown'
                  )
                "#,
                params![id, cutoff],
            )
            .map_err(|error| error.to_string())?;
        if removed == 1 {
            tx.execute(
                "DELETE FROM job_capabilities WHERE consumed_job_id = ?1",
                [id],
            )
            .map_err(|error| error.to_string())?;
        }
        tx.commit().map_err(|error| error.to_string())?;
        Ok(removed == 1)
    }
}
