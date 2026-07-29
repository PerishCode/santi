use santi_model::{ingest, message};

#[derive(Clone)]
pub struct InboxDraft<'a> {
    pub tag: &'a str,
    pub strand: &'a str,
    pub kind: message::Kind,
    pub content: &'a message::Content,
    pub source: Option<&'a ingest::Source>,
    pub created: &'a str,
}

#[derive(Clone, Copy)]
pub struct NoticeDraft<'a> {
    pub tag: &'a str,
    pub strand: &'a str,
    pub key: &'a str,
    pub revision: i64,
    pub digest: &'a str,
    pub content: &'a message::Content,
    pub source: &'a ingest::Source,
    pub causes: &'a [String],
    pub created: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub inbox: Option<String>,
    pub inserted: bool,
}

#[derive(Debug, Clone)]
pub struct Inbox {
    pub id: String,
    pub strand: String,
    pub kind: message::Kind,
    pub content: message::Content,
    pub source: Option<ingest::Source>,
    pub coalesce_key: Option<String>,
    pub coalesce_revision: Option<i64>,
    pub coalesce_causes: Vec<String>,
    pub created: String,
}

#[derive(Clone)]
pub struct DrainDraft<'a> {
    pub turn: &'a str,
    pub strand: &'a str,
    pub trigger: santi_model::turn::Trigger,
    pub source: Option<&'a str>,
    pub actor: &'a str,
    pub created: &'a str,
}

pub struct ReceiptDraft<'a> {
    pub inbox: &'a str,
    pub state: santi_model::receipt::State,
    pub turn: Option<&'a str>,
    pub incident: Option<&'a str>,
    pub rebuilt: Option<&'a str>,
    pub occurred: &'a str,
}

#[derive(Debug, Clone)]
pub struct Begun {
    pub turn: santi_model::turn::Turn,
    pub drained: Vec<message::Placed>,
}

#[derive(Debug, Clone)]
pub enum Opening {
    Started(Begun),
    Running(santi_model::turn::Turn),
    Idle,
}
