use santi_model::DownstreamCredential;

use super::SantiStore;
use super::db::Database;
use crate::timestamp_now;

impl SantiStore {
    pub fn create_downstream(
        &self,
        id: &str,
        label_prefix: &str,
        credential_env: &str,
    ) -> Result<DownstreamCredential, String> {
        let conn = self.conn.lock().unwrap();
        let now = timestamp_now();
        Database::new(&conn).insert_downstream(id, label_prefix, credential_env, &now)?;
        Database::new(&conn)
            .downstream_by_id(id)?
            .ok_or_else(|| "created downstream missing".to_string())
    }

    pub fn list_downstreams(&self) -> Result<Vec<DownstreamCredential>, String> {
        let conn = self.conn.lock().unwrap();
        Database::new(&conn).list_downstreams()
    }
}
