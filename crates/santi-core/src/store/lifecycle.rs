use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use rusqlite::{Connection, params};

use super::db::migrate;
use crate::store::DEFAULT_SOUL_ID;
use crate::store::SantiStore;
use crate::timestamp_now;

impl SantiStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let conn = Connection::open(path.as_ref()).map_err(|error| error.to_string())?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|error| error.to_string())?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        migrate(&store.conn.lock().unwrap())?;
        store.seed_defaults()?;
        Ok(store)
    }

    fn seed_defaults(&self) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        let now = timestamp_now();
        conn.execute(
            r#"
            INSERT OR IGNORE INTO souls (id, created_at, updated_at)
            VALUES (?1, ?2, ?2)
            "#,
            params![DEFAULT_SOUL_ID, now],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    }
}
