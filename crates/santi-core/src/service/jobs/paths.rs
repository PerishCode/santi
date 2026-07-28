use std::path::PathBuf;

use super::Service;
use santi_estate::JobRecord;

impl Service {
    pub(super) fn jobhome(&self, record: &JobRecord) -> PathBuf {
        let key = if record.generation.starts_with("stamp_") {
            &record.generation
        } else {
            &record.job.id
        };
        self.runtime().join("jobs").join(key)
    }
}
