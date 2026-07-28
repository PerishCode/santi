use std::time::{Duration, Instant};

use super::{Service, attention};

impl Service {
    fn sweep(&self) -> Result<(), String> {
        for record in self.store.active()? {
            let id = record.job.id.clone();
            let result = self
                .refresh(record)
                .and_then(|record| attention::capture(self, record));
            if let Err(error) = result {
                eprintln!("santi: job attention failed job={id} detail={error}");
            }
        }
        Ok(())
    }

    pub async fn watch(&self) {
        let mut next = Instant::now();
        while !self.closing() {
            if let Err(error) = self.sweep() {
                eprintln!("santi: job attention scan failed: {error}");
            }
            next = self.pace(next);
            self.rouse();
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}
