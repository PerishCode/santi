use super::super::DrainDraft;
use super::codec::Pending;
use keel::Row;

pub(super) enum Opened {
    Started(Vec<String>),
    Running(String),
    Idle,
}

#[derive(Clone)]
pub(super) struct Written {
    pub key: i64,
    pub tag: String,
    pub sequence: i64,
}

pub(super) struct Assigned {
    pub pending: Pending,
    pub message: Written,
}

pub(super) struct Message<'a, 'draft> {
    pub strand: &'a Row,
    pub kind: &'a str,
    pub content: &'a str,
    pub draft: &'a DrainDraft<'draft>,
}
