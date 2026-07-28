use keel::atom::{int, string};
use keel::resource;

#[resource]
pub(crate) struct Soul {
    #[field(string, unique)]
    tag: string,
    #[field(string)]
    created: string,
    #[field(string)]
    updated: string,
}

#[resource]
pub(crate) struct Strand {
    #[field(string, unique)]
    tag: string,
    #[field(string, unique = soul, opt)]
    label: string,
    #[field(string, default = "")]
    memory: string,
    #[field(string, opt)]
    state: string,
    #[field(int, default = 1, min = 1)]
    next: int,
    #[field(int, default = 0, min = 0)]
    seen: int,
    #[field(int, opt, min = 0)]
    fork: int,
    #[field(string)]
    created: string,
    #[field(string)]
    updated: string,
    #[relation(Soul, many2one, root)]
    soul: Soul,
    #[relation(Strand, many2one, opt)]
    parent: Strand,
}

#[resource]
pub(crate) struct Turn {
    #[field(string, unique)]
    tag: string,
    #[field(string, values = ("strand_send", "system"))]
    trigger: string,
    #[field(string, opt)]
    source: string,
    #[field(int, min = 0)]
    from: int,
    #[field(string)]
    created: string,
    #[field(string)]
    updated: string,
    #[relation(Strand, many2one, root)]
    strand: Strand,
}

#[resource(frozen)]
pub(crate) struct TurnCompletion {
    #[field(int, min = 0)]
    to: int,
    #[field(string)]
    finished: string,
    #[relation(Turn, one2one, root)]
    turn: Turn,
}

#[resource(frozen)]
pub(crate) struct TurnFailure {
    #[field(string)]
    error: string,
    #[field(string)]
    finished: string,
    #[relation(Turn, one2one, root)]
    turn: Turn,
}

#[resource]
pub(crate) struct Message {
    #[field(string, unique)]
    tag: string,
    #[field(string, values = ("soul", "system"))]
    actor_type: string,
    #[field(string)]
    actor: string,
    #[field(string, default = "text", values = ("santi_system", "text"))]
    kind: string,
    #[field(string)]
    content: string,
    #[field(string, values = ("aborted", "fixed", "pending"))]
    state: string,
    #[field(int, default = 1, min = 1)]
    version: int,
    #[field(bool, default = false)]
    request: bool,
    #[field(string, opt)]
    deleted: string,
    #[field(string)]
    created: string,
    #[field(string)]
    updated: string,
}

#[resource(frozen)]
pub(crate) struct StrandEntry {
    #[field(string, values = ("message", "thinking", "tool_call", "tool_result"))]
    target_type: string,
    #[field(string, unique = (strand, target_type))]
    target: string,
    #[field(int, unique = strand, min = 1)]
    sequence: int,
    #[field(string)]
    created: string,
    #[relation(Strand, many2one, root)]
    strand: Strand,
}

#[resource]
pub(crate) struct TurnStop {
    #[field(string, values = ("operator", "shutdown"))]
    cause: string,
    #[field(string)]
    requested: string,
    #[field(string, opt)]
    settled: string,
    #[relation(Turn, one2one, root)]
    turn: Turn,
}
