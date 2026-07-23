use std::{fs, path::Path};

use super::Service;
use crate::strand;

impl Service {
    pub fn fork_strand(&self, parent: &str) -> Result<strand::Forked, String> {
        let parent = self
            .store
            .strand(parent)?
            .ok_or_else(|| "parent strand not found".to_string())?;
        let fork = parent.next - 1;
        let child = self.store.fork_strand(&parent.id, fork)?;
        if let Err(error) = self.sync_fork_workspace(&parent.id, &child.id) {
            let child_memory_dir = self.strand_memory_dir(&child.id);
            if let Some(child_root) = child_memory_dir.parent() {
                let _ = fs::remove_dir_all(child_root);
            } else {
                let _ = fs::remove_dir_all(&child_memory_dir);
            }
            let _ = self.store.delete_fork_child_strand(&child.id);
            return Err(format!("fork workspace sync failed: {error}"));
        }
        Ok(strand::Forked { strand: child })
    }

    fn sync_fork_workspace(&self, parent: &str, child_strand_id: &str) -> Result<(), String> {
        let parent_dir = self.strand_memory_dir(parent);
        let child_dir = self.strand_memory_dir(child_strand_id);
        if child_dir.exists() {
            return Err(format!(
                "child strand workspace already exists: {}",
                child_dir.display()
            ));
        }
        if !parent_dir.exists() {
            fs::create_dir_all(&child_dir).map_err(|error| error.to_string())?;
            return Ok(());
        }
        copy_dir_all(&parent_dir, &child_dir)
    }
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(src).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        let child_src = entry.path();
        let child_dst = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_all(&child_src, &child_dst)?;
        } else if file_type.is_file() {
            fs::copy(&child_src, &child_dst).map_err(|error| error.to_string())?;
        } else if file_type.is_symlink() {
            return Err(format!(
                "cannot copy symlink in strand workspace: {}",
                child_src.display()
            ));
        }
    }
    Ok(())
}
