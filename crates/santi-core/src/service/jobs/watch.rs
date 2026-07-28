use std::time::{Duration, Instant};

use super::{Service, attention};

impl Service {
    async fn sweep(&self) -> Result<(), String> {
        for record in self.store.active_jobs().await? {
            let id = record.job.id.clone();
            let result = match self.refresh(record).await {
                Ok(record) => attention::capture(self, record).await,
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                eprintln!("santi: job attention failed job={id} detail={error}");
            }
        }
        Ok(())
    }

    pub async fn watch(&self) {
        let mut next = Instant::now();
        while !self.closing() {
            if let Err(error) = self.sweep().await {
                eprintln!("santi: job attention scan failed: {error}");
            }
            next = self.pace(next).await;
            self.rouse().await;
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}
