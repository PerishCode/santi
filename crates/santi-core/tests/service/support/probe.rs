use super::*;

pub(crate) struct Probe<'a> {
    service: &'a Service,
}

impl<'a> Probe<'a> {
    pub(crate) fn new(service: &'a Service) -> Self {
        Self { service }
    }

    async fn snapshot(&self, strand_id: &str) -> santi_core::StrandRuntimeSnapshot {
        self.service
            .runtime_snapshot(strand_id)
            .expect("runtime snapshot")
            .expect("strand runtime")
    }

    pub(crate) async fn any_completed(&self, strand_id: &str) -> santi_core::StrandRuntimeSnapshot {
        for _ in 0..50 {
            let runtime = self.snapshot(strand_id).await;
            if runtime
                .turns
                .iter()
                .any(|turn| turn.status == santi_core::TurnStatus::Completed)
            {
                return runtime;
            }
            sleep(Duration::from_millis(20)).await;
        }
        panic!("no turn completed");
    }

    pub(crate) async fn completed_count(
        &self,
        strand_id: &str,
        count: usize,
    ) -> santi_core::StrandRuntimeSnapshot {
        for _ in 0..50 {
            let runtime = self.snapshot(strand_id).await;
            if runtime
                .turns
                .iter()
                .filter(|turn| turn.status == santi_core::TurnStatus::Completed)
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
        strand_id: &str,
        turn_id: &str,
    ) -> santi_core::StrandRuntimeSnapshot {
        for _ in 0..50 {
            let runtime = self.snapshot(strand_id).await;
            if runtime
                .turns
                .iter()
                .any(|turn| turn.id == turn_id && turn.status == santi_core::TurnStatus::Completed)
            {
                return runtime;
            }
            sleep(Duration::from_millis(20)).await;
        }
        panic!("turn did not complete");
    }

    pub(crate) async fn failed_turn(
        &self,
        strand_id: &str,
        turn_id: &str,
    ) -> santi_core::StrandRuntimeSnapshot {
        for _ in 0..50 {
            let runtime = self.snapshot(strand_id).await;
            if runtime
                .turns
                .iter()
                .any(|turn| turn.id == turn_id && turn.status == santi_core::TurnStatus::Failed)
            {
                return runtime;
            }
            sleep(Duration::from_millis(20)).await;
        }
        panic!("turn did not fail");
    }

    pub(crate) async fn message_containing(
        &self,
        strand_id: &str,
        needle: &str,
    ) -> santi_core::StrandRuntimeSnapshot {
        for _ in 0..50 {
            let runtime = self.snapshot(strand_id).await;
            if runtime
                .messages
                .iter()
                .any(|message| message.content_text.contains(needle))
            {
                return runtime;
            }
            sleep(Duration::from_millis(20)).await;
        }
        panic!("message containing {needle:?} did not appear");
    }
}

pub(crate) fn count_messages(runtime: &santi_core::StrandRuntimeSnapshot, text: &str) -> usize {
    runtime
        .messages
        .iter()
        .filter(|message| message.content_text == text)
        .count()
}

pub(crate) fn provider_messages(request: &ProviderRequest) -> Vec<(&str, &str)> {
    request
        .input
        .iter()
        .filter_map(|item| match item {
            ProviderItem::Message { role, content } => Some((role.as_str(), content.as_str())),
            _ => None,
        })
        .collect()
}
