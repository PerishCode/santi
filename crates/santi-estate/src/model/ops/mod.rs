use super::job::{Job, StrandEffect, ToolCall};
use super::ledger::{Soul, Strand, Turn};
use keel::atom::{int, string};
use keel::resource;

pub(crate) mod error;

#[resource]
pub(crate) struct JobCapability {
    #[field(string, unique)]
    digest: string,
    #[field(int, min = 1)]
    expires: int,
    #[field(string, opt)]
    request_sha256: string,
    #[field(string)]
    created: string,
    #[relation(Soul, many2one)]
    soul: Soul,
    #[relation(Strand, many2one, root)]
    strand: Strand,
    #[relation(Turn, many2one)]
    turn: Turn,
    #[relation(ToolCall, many2one)]
    call: ToolCall,
    #[relation(StrandEffect, many2one)]
    effect: StrandEffect,
    #[relation(Job, one2one, opt)]
    consumed: Job,
}

#[resource(frozen)]
pub(crate) struct TraceRecord {
    #[field(string, unique)]
    tag: string,
    #[field(string)]
    boot: string,
    #[field(int, min = 1)]
    span: int,
    #[field(int, opt, min = 1)]
    parent: int,
    #[field(string)]
    name: string,
    #[field(string)]
    tags: string,
    #[field(string)]
    opened: string,
    #[field(string)]
    closed: string,
}

#[resource]
pub(crate) struct OutboxStream {
    #[field(string, unique)]
    tag: string,
}

#[resource(frozen)]
pub(crate) struct TurnOutbox {
    #[field(string, unique)]
    tag: string,
    #[field(serial, scope = stream)]
    sequence: int,
    #[field(string)]
    label: string,
    #[field(string)]
    payload: string,
    #[field(string)]
    created: string,
    #[relation(OutboxStream, many2one, root)]
    stream: OutboxStream,
    #[relation(Turn, one2one)]
    turn: Turn,
}
