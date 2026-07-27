use rusqlite::{OptionalExtension, params};

use super::Attention;
use crate::store::budget::notice;
use crate::store::{Notice, Offered, Store};

impl Store {
    pub(crate) fn attend(
        &self,
        attention: Attention<'_>,
        notice: Notice<'_>,
    ) -> Result<Offered, String> {
        let next = attention
            .base
            .checked_add(1)
            .ok_or_else(|| "job attention revision is out of range".to_string())?;
        if notice.revision != next {
            return Err("job attention and inbox revisions do not agree".to_string());
        }
        let base = i64::try_from(attention.base)
            .map_err(|_| "job attention revision is out of range".to_string())?;
        let revision = i64::try_from(next)
            .map_err(|_| "job attention revision is out of range".to_string())?;
        let tick = i64::try_from(attention.tick)
            .map_err(|_| "job reminder tick is out of range".to_string())?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let current = tx
            .query_row(
                "SELECT attention_revision FROM jobs WHERE id = ?1",
                [attention.id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "job not found".to_string())?;
        if current < base {
            return Err("job attention revision has a gap".to_string());
        }
        if current == base {
            tx.execute(
                r#"
                UPDATE jobs
                SET attention_revision = ?2,
                    runtime_warned_at = CASE
                        WHEN ?3 = 1 THEN COALESCE(runtime_warned_at, ?6)
                        ELSE runtime_warned_at
                    END,
                    output_warned_at = CASE
                        WHEN ?4 = 1 THEN COALESCE(output_warned_at, ?6)
                        ELSE output_warned_at
                    END,
                    last_reminded_at = CASE
                        WHEN ?5 = 1 THEN ?6
                        ELSE last_reminded_at
                    END,
                    next_reminder_at = CASE
                        WHEN ?5 = 1 THEN ?7
                        ELSE next_reminder_at
                    END,
                    reminder_tick = CASE
                        WHEN ?5 = 1 THEN ?8
                        ELSE reminder_tick
                    END,
                    updated_at = ?6
                WHERE id = ?1 AND attention_revision = ?9
                "#,
                params![
                    attention.id,
                    revision,
                    attention.runtime as i64,
                    attention.output as i64,
                    attention.reminded as i64,
                    attention.at,
                    attention.next,
                    tick,
                    base
                ],
            )
            .map_err(|error| error.to_string())?;
        }
        let offered = notice::stow(&tx, notice)?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(offered)
    }
}
