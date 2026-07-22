use rusqlite::{OptionalExtension, params};
use santi_model::DownstreamCredential;

use super::Database;
use crate::rows::{Decode, collect_rows};

impl Database<'_> {
    pub fn insert_downstream(
        &self,
        id: &str,
        label_prefix: &str,
        credential_env: &str,
        created_at: &str,
    ) -> Result<(), String> {
        self.conn
            .execute(
                r#"
                INSERT INTO downstreams (id, label_prefix, credential_env, created_at, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?4)
                "#,
                params![id, label_prefix, credential_env, created_at],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn downstream_by_id(&self, id: &str) -> Result<Option<DownstreamCredential>, String> {
        self.conn
            .query_row(
                r#"
                SELECT id, label_prefix, credential_env, created_at, updated_at
                FROM downstreams WHERE id = ?1 LIMIT 1
                "#,
                params![id],
                DownstreamCredential::decode,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn list_downstreams(&self) -> Result<Vec<DownstreamCredential>, String> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
                SELECT id, label_prefix, credential_env, created_at, updated_at
                FROM downstreams ORDER BY created_at ASC, id ASC
                "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map([], DownstreamCredential::decode)
            .map_err(|error| error.to_string())?;
        collect_rows(rows)
    }
}
