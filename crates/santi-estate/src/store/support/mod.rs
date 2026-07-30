mod bootstrap;
mod environ;
pub(super) mod read;
mod trace;
pub(super) mod transition;
pub(super) mod write;

use keel::adapt::db::Sqlite;
use std::path::Path;

pub use bootstrap::{Bootstrap, Status};
pub use environ::EnvironDraft;
pub use trace::TraceDraft;

pub(super) async fn wire(path: &Path) -> Result<Sqlite, String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    if let Some(quarantine) = transition::prepare(path).await? {
        eprintln!(
            "santi-estate: quarantined legacy database at {}",
            quarantine.display()
        );
    }
    Sqlite::file(path).await.map_err(read::error)
}
