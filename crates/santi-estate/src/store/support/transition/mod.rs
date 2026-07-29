use std::path::{Path, PathBuf};

mod database;
mod manifest;
mod quarantine;
mod signature;

use manifest::Manifest;
use quarantine::Estate;

pub(in crate::store) async fn prepare(path: &Path) -> Result<Option<PathBuf>, String> {
    let estate = Estate::new(path);
    let _lock = estate.lock()?;
    if let Some((dir, held)) = estate.pending()? {
        return estate.move_files(&dir, held).map(Some);
    }
    if !path.exists() {
        estate.refuse_orphans()?;
        return Ok(None);
    }
    estate.refuse_non_file()?;
    let (version, objects) = database::probe(path).await?;
    if version == 0 {
        return Ok(None);
    }
    if version != signature::VERSION {
        return Err(format!(
            "unsupported legacy database version {version}; expected {}; database left unchanged",
            signature::VERSION
        ));
    }
    if !signature::exact(&objects) {
        return Err("legacy database v39 shape is not exact; database left unchanged".to_string());
    }
    database::exclusive(path).await?;
    let dir = estate.generation()?;
    let manifest = Manifest::moving(estate.source()?, estate.files()?);
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    manifest.write(&dir)?;
    estate.move_files(&dir, manifest).map(Some)
}
