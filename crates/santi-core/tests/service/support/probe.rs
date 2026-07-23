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

    pub(crate) async fn completed_count(
        &self,
        strand: &str,
        count: usize,
    ) -> santi_core::stream::Snapshot {
        for _ in 0..50 {
            let runtime = self.snapshot(strand).await;
            if runtime
                .turns
                .iter()
                .filter(|turn| turn.status == santi_core::turn::Status::Completed)
                .count()
                >= count
            {
                return runtime;
            }
            sleep(Duration::from_millis(20)).await;
        }
        panic!("{count} turns did not complete");
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

    pub(crate) async fn failed_turn(
        &self,
        strand: &str,
        turn: &str,
    ) -> santi_core::stream::Snapshot {
        for _ in 0..50 {
            let runtime = self.snapshot(strand).await;
            if runtime
                .turns
                .iter()
                .any(|held| held.id == turn && held.status == santi_core::turn::Status::Failed)
            {
                return runtime;
            }
            sleep(Duration::from_millis(20)).await;
        }
        panic!("turn did not fail");
    }

    pub(crate) async fn message_containing(
        &self,
        strand: &str,
        needle: &str,
    ) -> santi_core::stream::Snapshot {
        for _ in 0..50 {
            let runtime = self.snapshot(strand).await;
            if runtime
                .messages
                .iter()
                .any(|message| message.text.contains(needle))
            {
                return runtime;
            }
            sleep(Duration::from_millis(20)).await;
        }
        panic!("message containing {needle:?} did not appear");
    }
}

pub(crate) fn count_messages(runtime: &santi_core::stream::Snapshot, text: &str) -> usize {
    runtime
        .messages
        .iter()
        .filter(|message| message.text == text)
        .count()
}

pub(crate) fn provider_messages(request: &Request) -> Vec<(&str, &str)> {
    request
        .input
        .iter()
        .filter_map(|item| match item {
            Item::Message { role, content } => Some((role.as_str(), content.as_str())),
            _ => None,
        })
        .collect()
}
