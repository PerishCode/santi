use rusqlite::{OptionalExtension, params};

use super::Database;
use crate::rows::{Decode, collected};
use santi_model::downstream;

pub struct Stowed<'a> {
    pub owner: &'a str,
    pub request: &'a str,
    pub digest: &'a str,
    pub strand: &'a str,
    pub inbox: &'a str,
    pub created: &'a str,
}

impl Database<'_> {
    pub fn enroll(
        &self,
        id: &str,
        prefix: &str,
        digest: &str,
        created: &str,
    ) -> Result<(), String> {
        self.conn
            .execute(
                r#"
                INSERT INTO downstreams (
                  id, label_prefix, credential_sha256, created_at, updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?4)
                "#,
                params![id, prefix, digest, created],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn downstream(&self, id: &str) -> Result<Option<downstream::Credential>, String> {
        self.conn
            .query_row(
                r#"
                SELECT id, label_prefix, credential_sha256, created_at, updated_at
                FROM downstreams WHERE id = ?1 LIMIT 1
                "#,
                params![id],
                downstream::Credential::decode,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn downstreams(&self) -> Result<Vec<downstream::Credential>, String> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
                SELECT id, label_prefix, credential_sha256, created_at, updated_at
                FROM downstreams ORDER BY created_at ASC, id ASC
                "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map([], downstream::Credential::decode)
            .map_err(|error| error.to_string())?;
        collected(rows)
    }

    pub fn replay(
        &self,
        owner: &str,
        request: &str,
    ) -> Result<Option<(String, String, String)>, String> {
        self.conn
            .query_row(
                r#"
                SELECT request_sha256, strand_id, inbox_id
                FROM downstream_ingest
                WHERE downstream_id = ?1 AND request_id = ?2
                "#,
                params![owner, request],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn stow(&self, input: Stowed<'_>) -> Result<(), String> {
        self.conn
            .execute(
                r#"
                INSERT INTO downstream_ingest (
                  downstream_id, request_id, request_sha256,
                  strand_id, inbox_id, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                "#,
                params![
                    input.owner,
                    input.request,
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
