use std::{fs, path::Path};

use super::Service;
use crate::strand;

impl Service {
    pub fn fork(&self, parent: &str) -> Result<strand::Forked, String> {
        let parent = self
            .store
            .strand(parent)?
            .ok_or_else(|| "parent strand not found".to_string())?;
        let fork = parent.next - 1;
        let child = self.store.fork(&parent.id, fork)?;
        if let Err(error) = self.sync_fork_workspace(&parent.id, &child.id) {
            let nursery = self.strand_memory_dir(&child.id);
            if let Some(child_root) = nursery.parent() {
                let _ = fs::remove_dir_all(child_root);
            } else {
                let _ = fs::remove_dir_all(&nursery);
            }
            let _ = self.store.delete_fork_child_strand(&child.id);
            return Err(format!("fork workspace sync failed: {error}"));
        }
        Ok(strand::Forked { strand: child })
    }

    fn sync_fork_workspace(&self, parent: &str, child_strand_id: &str) -> Result<(), String> {
        let parent = self.strand_memory_dir(parent);
        let dir = self.strand_memory_dir(child_strand_id);
        if dir.exists() {
            return Err(format!(
                "child strand workspace already exists: {}",
                dir.display()
            ));
        }
        if !parent.exists() {
            fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
            return Ok(());
        }
        copied(&parent, &dir)
    }
}

fn copied(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(src).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let kind = entry.file_type().map_err(|error| error.to_string())?;
        let src = entry.path();
        let dst = dst.join(entry.file_name());
        if kind.is_dir() {
            copied(&src, &dst)?;
        } else if kind.is_file() {
            fs::copy(&src, &dst).map_err(|error| error.to_string())?;
        } else if kind.is_symlink() {
            return Err(format!(
                "cannot copy symlink in strand workspace: {}",
                src.display()
            ));
        }
    }
    Ok(())
}
