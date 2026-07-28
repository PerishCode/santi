use super::*;

pub(crate) struct Probe<'a> {
    service: &'a Service,
}

impl<'a> Probe<'a> {
    pub(crate) fn new(service: &'a Service) -> Self {
        Self { service }
    }

    async fn snapshot(&self, strand: &str) -> santi_core::stream::Snapshot {
        self.service
            .snapshot(strand)
            .await
            .expect("runtime snapshot")
            .expect("strand runtime")
    }

    pub(crate) async fn any_completed(&self, strand: &str) -> santi_core::stream::Snapshot {
        for _ in 0..50 {
            let runtime = self.snapshot(strand).await;
            if runtime
                .turns
                .iter()
                .any(|turn| turn.status == santi_core::turn::Status::Completed)
            {
                return runtime;
            }
            sleep(Duration::from_millis(20)).await;
        }
        panic!("no turn completed");
    }

    pub(crate) async fn completed_turn(
        &self,
        strand: &str,
        turn: &str,
    ) -> santi_core::stream::Snapshot {
        for _ in 0..50 {
            let runtime = self.snapshot(strand).await;
            if runtime
                .turns
                .iter()
                .any(|held| held.id == turn && held.status == santi_core::turn::Status::Completed)
            {
                return runtime;
            }
            sleep(Duration::from_millis(20)).await;
        }
        panic!("turn did not complete");
    }
}
