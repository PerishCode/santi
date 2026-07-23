pub(crate) use rusqlite::Connection;
use santi_core::message;
pub(crate) use santi_core::{Completion, Draft, Invocation, Item, Store};
pub(crate) use serde_json::json;

pub(crate) fn assert_text(item: &Item, role: &str, content: &str) {
    match item {
        Item::Message {
            role: actual_role,
            content: actual_content,
        } => {
            assert_eq!(actual_role, role);
            assert_eq!(actual_content, content);
        }
        other => panic!("expected text item, got {other:?}"),
    }
}

pub(crate) struct Line<'a> {
    pub store: &'a Store,
    pub strand: &'a str,
    pub actor: message::Role,
    pub text: &'a str,
    pub intake: message::Intake,
}

pub(crate) fn append_timeline_message(line: Line<'_>) {
    match line.intake {
        message::Intake::Request => {
            line.store
                .enqueue_inbox(
                    line.strand,
                    message::Kind::Text,
                    message::Content::text(line.text),
                )
                .expect("enqueue inbox");
        }
        message::Intake::Record => {
            let actor = match line.actor {
                message::Role::Soul => line.store.default_soul_id(),
                message::Role::System => line.store.system(),
            };
            line.store
                .append_message(Draft {
                    strand: line.strand,
                    actor: line.actor,
                    id: actor,
                    content: message::Content::text(line.text),
                    state: message::State::Fixed,
                    intake: line.intake,
                })
                .expect("append message");
        }
    }
}
