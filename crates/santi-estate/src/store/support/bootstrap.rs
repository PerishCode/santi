use std::path::Path;

use keel::adapt::db::Sqlite;

use super::{read, wire};
use crate::store::Store;

pub struct Bootstrap {
    held: keel::Bootstrap<Sqlite>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    Vacant,
    Occupied,
}

impl Bootstrap {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let held =
            keel::bootstrap(crate::graph(), wire(path.as_ref()).await?).map_err(read::error)?;
        Ok(Self { held })
    }

    pub async fn status(&mut self) -> Result<Status, String> {
        self.held
            .status()
            .await
            .map(|status| match status {
                keel::Status::Vacant => Status::Vacant,
                keel::Status::Occupied => Status::Occupied,
            })
            .map_err(read::error)
    }

    pub async fn mint(&mut self) -> Result<String, String> {
        self.held.mint().await.map_err(read::error)
    }

    pub async fn seal(self, sudo: &str) -> Result<Store, String> {
        let core = self.held.seal(sudo).await.map_err(read::error)?.share();
        Ok(Store { core })
    }
}

impl Store {
    pub async fn bootstrap(path: impl AsRef<Path>, sudo: &str) -> Result<Self, String> {
        Bootstrap::open(path).await?.seal(sudo).await
    }
}
