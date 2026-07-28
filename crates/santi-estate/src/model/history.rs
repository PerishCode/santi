use super::job::ToolCall;
use super::ledger::{Message, Strand, Turn};
use keel::atom::{int, string};
use keel::resource;

#[resource(frozen)]
pub(crate) struct MessageEvent {
    #[field(string, unique)]
    tag: string,
    #[field(string, values = ("delete", "fix", "insert", "patch", "remove"))]
    action: string,
    #[field(string, values = ("soul", "system"))]
    actor_type: string,
    #[field(string)]
    actor: string,
    #[field(int, min = 1)]
    base_version: int,
    #[field(string)]
    payload: string,
    #[field(string)]
    created: string,
    #[relation(Message, many2one, root)]
    message: Message,
}

#[resource(frozen)]
pub(crate) struct ToolResult {
    #[field(string, unique)]
    tag: string,
    #[field(string)]
    created: string,
    #[relation(ToolCall, one2one, root)]
    call: ToolCall,
}

#[resource(frozen)]
pub(crate) struct ToolOutput {
    #[field(string)]
    output: string,
    #[relation(ToolResult, one2one, root)]
    result: ToolResult,
}

#[resource(frozen)]
pub(crate) struct ToolFailure {
    #[field(string)]
    error: string,
    #[relation(ToolResult, one2one, root)]
    result: ToolResult,
}

#[resource]
pub(crate) struct ThinkingSpan {
    #[field(string, unique)]
    tag: string,
    #[field(string, opt)]
    response: string,
    #[field(string, opt)]
    summary: string,
    #[field(string)]
    created: string,
    #[field(string)]
    updated: string,
    #[relation(Turn, many2one, root)]
    turn: Turn,
}

#[resource(frozen)]
pub(crate) struct ThinkingCompletion {
    #[field(string, values = ("called", "finished", "spoke"))]
    reason: string,
    #[field(string)]
    finished: string,
    #[relation(ThinkingSpan, one2one, root)]
    thinking: ThinkingSpan,
}

#[resource(frozen)]
pub(crate) struct ThinkingFailure {
    #[field(string)]
    error: string,
    #[field(string)]
    finished: string,
    #[relation(ThinkingSpan, one2one, root)]
    thinking: ThinkingSpan,
}

#[resource(frozen)]
pub(crate) struct Compact {
    #[field(string, unique)]
    tag: string,
    #[field(string)]
    summary: string,
    #[field(string, opt)]
    created: string,
    #[field(string, opt)]
    metadata: string,
    #[relation(Strand, many2one, root)]
    strand: Strand,
    #[relation(Message, many2one)]
    first: Message,
    #[relation(Message, many2one)]
    last: Message,
}
