use super::SantiStore;
use super::db::Database;
use crate::now;
use crate::{downstream, ingest};

impl SantiStore {
    pub fn create_downstream(
        &self,
        id: &str,
        prefix: &str,
        digest: &str,
    ) -> Result<downstream::Credential, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let downstreams = Database::new(&tx).list_downstreams()?;
        if let Some(existing) = downstreams.iter().find(|downstream| downstream.id == id) {
            if existing.prefix == prefix && existing.digest == digest {
                tx.commit().map_err(|error| error.to_string())?;
                return Ok(existing.clone());
            }
            return Err("downstream id conflicts with an existing registration".to_string());
        }
        if downstreams.iter().any(|downstream| {
            downstream.prefix.starts_with(prefix) || prefix.starts_with(&downstream.prefix)
        }) {
            return Err("downstream prefix overlaps an existing registration".to_string());
        }
        if downstreams
            .iter()
            .any(|downstream| downstream.digest == digest)
        {
            return Err("downstream digest is already registered".to_string());
        }
        let now = now();
        Database::new(&tx).insert_downstream(id, prefix, digest, &now)?;
        let downstream = Database::new(&tx)
            .downstream_by_id(id)?
            .ok_or_else(|| "created downstream missing".to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(downstream)
    }

    pub fn list_downstreams(&self) -> Result<Vec<downstream::Credential>, String> {
        let conn = self.conn.lock().unwrap();
        Database::new(&conn).list_downstreams()
    }

    pub(crate) fn replay_downstream(
        &self,
        owner: &str,
        request: &str,
        digest: &str,
    ) -> Result<Option<ingest::Receipt>, String> {
        let conn = self.conn.lock().unwrap();
        let Some((accepted, strand, inbox)) = Database::new(&conn).replay(owner, request)? else {
            return Ok(None);
        };
        if accepted != digest {
            return Err("downstream request conflicts with an accepted payload".to_string());
        }
        Ok(Some(ingest::Receipt {
            strand,
            inbox,
            warning: None,
        }))
    }
}
