use santi_model::{DownstreamCredential, IngestReceipt};

use super::SantiStore;
use super::db::Database;
use crate::timestamp_now;

impl SantiStore {
    pub fn create_downstream(
        &self,
        id: &str,
        label_prefix: &str,
        credential_sha256: &str,
    ) -> Result<DownstreamCredential, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let downstreams = Database::new(&tx).list_downstreams()?;
        if let Some(existing) = downstreams.iter().find(|downstream| downstream.id == id) {
            if existing.label_prefix == label_prefix
                && existing.credential_sha256 == credential_sha256
            {
                tx.commit().map_err(|error| error.to_string())?;
                return Ok(existing.clone());
            }
            return Err("downstream id conflicts with an existing registration".to_string());
        }
        if downstreams.iter().any(|downstream| {
            downstream.label_prefix.starts_with(label_prefix)
                || label_prefix.starts_with(&downstream.label_prefix)
        }) {
            return Err("downstream label_prefix overlaps an existing registration".to_string());
        }
        if downstreams
            .iter()
            .any(|downstream| downstream.credential_sha256 == credential_sha256)
        {
            return Err("downstream credential_sha256 is already registered".to_string());
        }
        let now = timestamp_now();
        Database::new(&tx).insert_downstream(id, label_prefix, credential_sha256, &now)?;
        let downstream = Database::new(&tx)
            .downstream_by_id(id)?
            .ok_or_else(|| "created downstream missing".to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(downstream)
    }

    pub fn list_downstreams(&self) -> Result<Vec<DownstreamCredential>, String> {
        let conn = self.conn.lock().unwrap();
        Database::new(&conn).list_downstreams()
    }

    pub(crate) fn replay_downstream(
        &self,
        owner: &str,
        request: &str,
        digest: &str,
    ) -> Result<Option<IngestReceipt>, String> {
        let conn = self.conn.lock().unwrap();
        let Some((accepted, strand_id, inbox_id)) = Database::new(&conn).replay(owner, request)?
        else {
            return Ok(None);
        };
        if accepted != digest {
            return Err("downstream request conflicts with an accepted payload".to_string());
        }
        Ok(Some(IngestReceipt {
            strand_id,
            inbox_id,
            warning: None,
        }))
    }
}
