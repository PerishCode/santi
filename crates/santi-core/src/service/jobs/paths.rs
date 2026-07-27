use std::path::PathBuf;

use super::Service;
use crate::store::JobRecord;

impl Service {
    pub(super) fn jobhome(&self, record: &JobRecord) -> PathBuf {
        let key = if record.stamp.starts_with("stamp_") {
            &record.stamp
        } else {
            &record.job.id
        };
        self.runtime().join("jobs").join(key)
    }
}
