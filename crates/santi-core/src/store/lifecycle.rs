use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use rusqlite::{Connection, params};

use super::db::migrate;
use crate::now;
use crate::store::GENESIS;
use crate::store::Store;

impl Store {
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
        store.seeded()?;
        Ok(store)
    }

    fn seeded(&self) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        let now = now();
        conn.execute(
            r#"
            INSERT OR IGNORE INTO souls (id, created_at, updated_at)
            VALUES (?1, ?2, ?2)
            "#,
            params![GENESIS, now],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    }
}
