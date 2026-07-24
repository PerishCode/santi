use std::sync::{Arc, Mutex, mpsc};

use plumb::trace::{Engine, Record};
use rusqlite::Connection;

use crate::store::db::{Database, Recorded};
use crate::{stamped, tag, trace};

pub(in crate::store) struct Archive {
    sender: mpsc::Sender<Record>,
}

impl Archive {
    pub(in crate::store) fn open(conn: Arc<Mutex<Connection>>) -> Archive {
        let (sender, receiver) = mpsc::channel::<Record>();
        let boot = tag("boot");
        std::thread::spawn(move || {
            while let Ok(record) = receiver.recv() {
                let (Ok(opened), Ok(closed)) = (stamped(record.opened), stamped(record.closed))
                else {
                    continue;
                };
                let tags = record
                    .tags
                    .iter()
                    .map(|(key, value)| trace::Tag {
                        key: key.clone(),
                        value: value.clone(),
                    })
                    .collect::<Vec<_>>();
                let Ok(tags) = serde_json::to_string(&tags) else {
                    continue;
                };
                let held = conn.lock().unwrap();
                let _ = Database::new(&held).recorded(Recorded {
                    boot: &boot,
                    span: record.id as i64,
                    parent: record.parent.map(|parent| parent as i64),
                    name: &record.name,
                    tags: &tags,
                    opened: &opened,
                    closed: &closed,
                });
            }
        });
        Archive { sender }
    }
}

impl Engine for Archive {
    fn accept(&self, record: Record) {
        let _ = self.sender.send(record);
    }
}
