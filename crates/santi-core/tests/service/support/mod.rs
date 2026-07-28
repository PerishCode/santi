pub(crate) use async_trait::async_trait;
pub(crate) use futures_util::stream;
use santi_core::service::Service;
use santi_core::strand;
pub(crate) use santi_core::{SOULSPACE, STRANDSPACE, soulward, strandward};

mod probe;
pub(crate) use probe::*;

pub(crate) fn accepted_turn(response: &strand::Posted) -> &santi_core::turn::Turn {
    response.turn.as_ref().expect("send should land on a turn")
}
pub(crate) use santi_provider::{Event, Item, Metadata, Provider, Request, Streaming};
pub(crate) use std::sync::{Arc, Mutex};
pub(crate) use tokio::time::{Duration, sleep};

#[derive(Clone, Default)]
pub(crate) struct FakeProvider {
    pub(crate) requests: Arc<Mutex<Vec<Request>>>,
}

#[async_trait]
impl Provider for FakeProvider {
    fn metadata(&self) -> Metadata {
        Metadata {
            provider: Arc::from("fake-provider"),
            model: "fake-model".to_string(),
            budget: None,
        }
    }

    async fn stream(&self, request: Request) -> Result<Streaming, String> {
        {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request);
        }
        Ok(Box::pin(stream::iter(vec![
            Ok(Event::Text("hi from runtime".to_string())),
            Ok(Event::Completed {
                response: Some("fake-response-id".to_string()),
            }),
        ])))
    }
}
