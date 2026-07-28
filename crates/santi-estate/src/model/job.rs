use super::ledger::{Strand, Turn};
use keel::atom::{int, string};
use keel::resource;

#[resource(frozen)]
pub(crate) struct ToolCall {
    #[field(string, unique)]
    tag: string,
    #[field(string)]
    tool: string,
    #[field(string)]
    arguments: string,
    #[field(string)]
    created: string,
    #[relation(Turn, many2one, root)]
    turn: Turn,
}

#[resource]
pub(crate) struct StrandEffect {
    #[field(string, unique)]
    tag: string,
    #[field(string)]
    effect_type: string,
    #[field(
        string,
        default = "prepared",
        values = ("dispatching", "prepared", "settled_applied", "settled_not_applied", "unknown")
    )]
    state: string,
    #[field(string, opt)]
    result: string,
    #[field(string, opt)]
    error: string,
    #[field(string, opt)]
    metadata: string,
    #[field(string)]
    created: string,
    #[field(string)]
    updated: string,
    #[field(string, opt)]
    dispatched: string,
    #[field(string, opt)]
    settled: string,
    #[relation(Turn, many2one, root)]
    turn: Turn,
    #[relation(ToolCall, one2one, opt)]
    call: ToolCall,
}

#[resource]
pub(crate) struct Job {
    #[field(string, unique)]
    tag: string,
    #[field(string)]
    description: string,
    #[field(string)]
    command: string,
    #[field(string, opt)]
    cwd: string,
    #[field(int, min = 1)]
    timeout_seconds: int,
    #[field(int, min = 1)]
    output_limit_bytes: int,
    #[field(int, opt, min = 1)]
    remind_every_seconds: int,
    #[field(string)]
    request_sha256: string,
    #[field(string, unique)]
    capability_sha256: string,
    #[field(string, unique)]
    generation: string,
    #[field(string, unique)]
    supervisor_ref: string,
    #[field(
        string,
        default = "submitting",
        values = (
            "accepted",
            "cancelled",
            "cancelling",
            "failed",
            "running",
            "submitting",
            "succeeded",
            "timed_out",
            "unknown"
        )
    )]
    state: string,
    #[field(string, opt)]
    reason: string,
    #[field(int, opt)]
    exit_code: int,
    #[field(string)]
    created: string,
    #[field(string)]
    updated: string,
    #[field(string, opt)]
    accepted: string,
    #[field(string, opt)]
    started: string,
    #[field(int, opt, min = 0)]
    started_millis: int,
    #[field(string, opt)]
    finished: string,
    #[field(string, opt)]
    acknowledged: string,
    #[field(int, default = 0, min = 0)]
    attention_revision: int,
    #[field(string, opt)]
    runtime_warned: string,
    #[field(string, opt)]
    output_warned: string,
    #[field(string, opt)]
    last_reminded: string,
    #[field(string, opt)]
    next_reminder: string,
    #[field(int, default = 0, min = 0)]
    reminder_tick: int,
    #[relation(Strand, many2one, root)]
    strand: Strand,
    #[relation(Turn, many2one)]
    turn: Turn,
    #[relation(ToolCall, many2one)]
    call: ToolCall,
    #[relation(StrandEffect, many2one)]
    effect: StrandEffect,
}
