use rusqlite::{OptionalExtension, params};

use super::Database;

pub struct Stowed<'a> {
    pub subscription: &'a str,
    pub delivery: &'a str,
    pub digest: &'a str,
    pub strand: &'a str,
    pub inbox: &'a str,
    pub created: &'a str,
}

impl Database<'_> {
    pub fn delivery(
        &self,
        subscription: &str,
        delivery: &str,
    ) -> Result<Option<(String, String, String)>, String> {
        self.conn
            .query_row(
                r#"
                SELECT request_sha256, strand_id, inbox_id
                FROM webhook_deliveries
                WHERE subscription_name = ?1 AND delivery_id = ?2
                "#,
                params![subscription, delivery],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn deliver(&self, input: Stowed<'_>) -> Result<(), String> {
        self.conn
            .execute(
                r#"
                INSERT INTO webhook_deliveries (
                  subscription_name, delivery_id, request_sha256,
                  strand_id, inbox_id, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                "#,
                params![
                    input.subscription,
                    input.delivery,
                    input.digest,
                    input.strand,
                    input.inbox,
                    input.created
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}
