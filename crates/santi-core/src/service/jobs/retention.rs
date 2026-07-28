use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use super::Service;
use crate::stamped;
use santi_estate::ExpiredJob;

const BATCH: usize = 128;
const TRASH: &str = ".gc";

impl Service {
    pub fn retain(mut self, retention: Duration) -> Result<Self, String> {
        if retention.is_zero() {
            return Err("acknowledged job retention must be greater than zero".to_string());
        }
        self.retention = retention;
        Ok(self)
    }

    pub(super) async fn reap(&self) -> Result<bool, String> {
        if self.recover().await? {
            return Ok(true);
        }
        let cutoff = stamped(
            SystemTime::now()
                .checked_sub(self.retention)
                .unwrap_or(UNIX_EPOCH),
        )?;
        let expired = self.store.expired_jobs(&cutoff, BATCH).await?;
        let full = expired.len() == BATCH;
        for job in expired {
            if let Err(error) = self.collect(&job, &cutoff).await {
                eprintln!(
                    "santi: acknowledged job collection failed job={} detail={error}",
                    job.id
                );
            }
        }
        Ok(full)
    }

    pub(super) async fn pace(&self, next: Instant) -> Instant {
        let now = Instant::now();
        if now < next {
            return next;
        }
        match self.reap().await {
            Ok(true) => now + Duration::from_secs(1),
            Ok(false) => now + Duration::from_secs(60 * 60),
            Err(error) => {
                eprintln!("santi: acknowledged job collection failed: {error}");
                now + Duration::from_secs(60)
            }
        }
    }

    async fn recover(&self) -> Result<bool, String> {
        let trash = self.trash();
        let entries = match fs::read_dir(&trash) {
            Ok(entries) => entries.take(BATCH).collect::<Result<Vec<_>, _>>(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.to_string()),
        }
        .map_err(|error| error.to_string())?;
        let full = entries.len() == BATCH;
        for entry in entries {
            let key = entry
                .file_name()
                .into_string()
                .map_err(|_| "job collection key is not valid UTF-8".to_string())?;
            let archived = entry.path();
            if self.store.retained_job(&key).await? {
                let canonical = self.jobroot().join(&key);
                if canonical.exists() {
                    return Err(format!(
                        "job collection recovery conflicts with retained path {}",
                        canonical.display()
                    ));
                }
                fs::rename(&archived, canonical).map_err(|error| error.to_string())?;
            } else {
                remove(&archived)?;
            }
        }
        Ok(full)
    }

    async fn collect(&self, job: &ExpiredJob, cutoff: &str) -> Result<(), String> {
        let canonical = self.jobroot().join(&job.key);
        let archived = self.trash().join(&job.key);
        let moved = if canonical.exists() {
            fs::create_dir_all(self.trash()).map_err(|error| error.to_string())?;
            if archived.exists() {
                return Err(format!(
                    "job collection archive already exists {}",
                    archived.display()
                ));
            }
            fs::rename(&canonical, &archived).map_err(|error| error.to_string())?;
            true
        } else {
            false
        };
        let removed = match self.store.purge_job(&job.id, cutoff).await {
            Ok(removed) => removed,
            Err(error) => {
                if moved {
                    restore(&archived, &canonical)?;
                }
                return Err(error);
            }
        };
        if removed {
            if archived.exists() {
                remove(&archived)?;
            }
        } else if moved {
            restore(&archived, &canonical)?;
        }
        Ok(())
    }

    fn jobroot(&self) -> PathBuf {
        self.runtime().join("jobs")
    }

    fn trash(&self) -> PathBuf {
        self.jobroot().join(TRASH)
    }
}

fn restore(archived: &Path, canonical: &Path) -> Result<(), String> {
    if canonical.exists() {
        return Err(format!(
            "job collection restore conflicts with {}",
            canonical.display()
        ));
    }
    fs::rename(archived, canonical).map_err(|error| error.to_string())
}

fn remove(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.is_dir() {
        fs::remove_dir_all(path).map_err(|error| error.to_string())
    } else {
        fs::remove_file(path).map_err(|error| error.to_string())
    }
}
