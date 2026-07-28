use std::sync::Arc;

use plumb::trace::{Engine, Record};
use tokio::sync::mpsc;

use santi_estate::{Store, TraceDraft};

pub(super) struct Writer {
    sender: mpsc::UnboundedSender<Record>,
}

impl Writer {
    pub(super) fn start(store: Store) -> Arc<Self> {
        let (sender, mut receiver) = mpsc::unbounded_channel::<Record>();
        let boot = crate::tag("boot");
        tokio::spawn(async move {
            while let Some(record) = receiver.recv().await {
                let (Ok(opened), Ok(closed)) =
                    (crate::stamped(record.opened), crate::stamped(record.closed))
                else {
                    continue;
                };
                let tags = record
                    .tags
                    .iter()
                    .map(|(key, value)| crate::trace::Tag {
                        key: key.clone(),
                        value: value.clone(),
                    })
                    .collect::<Vec<_>>();
                if let Err(error) = store
                    .record_trace(TraceDraft {
                        tag: &crate::tag("trace"),
                        boot: &boot,
                        span: record.id as i64,
                        parent: record.parent.map(|parent| parent as i64),
                        name: &record.name,
                        tags: &tags,
                        opened: &opened,
                        closed: &closed,
                    })
                    .await
                {
                    eprintln!("santi: trace persistence failed: {error}");
                }
            }
        });
        Arc::new(Self { sender })
    }
}

impl Engine for Writer {
    fn accept(&self, record: Record) {
        let _ = self.sender.send(record);
    }
}
