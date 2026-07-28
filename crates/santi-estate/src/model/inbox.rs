use super::ledger::{Strand, Turn};
use keel::atom::{int, string};
use keel::resource;

#[resource]
pub(crate) struct StrandInbox {
    #[field(string, unique)]
    tag: string,
    #[field(string, values = ("santi_system", "text"))]
    kind: string,
    #[field(string)]
    content: string,
    #[field(string, opt)]
    source_type: string,
    #[field(string, opt)]
    source_ref: string,
    #[field(string, opt)]
    source_metadata: string,
    #[field(string, unique = strand, opt)]
    coalesce_key: string,
    #[field(int, opt, min = 1)]
    coalesce_revision: int,
    #[field(string, opt)]
    coalesce_causes: string,
    #[field(string)]
    created: string,
    #[relation(Strand, many2one, root)]
    strand: Strand,
}

#[resource]
pub(crate) struct InboxSlot {
    #[field(string, unique = strand)]
    key: string,
    #[field(int, min = 1)]
    revision: int,
    #[field(string)]
    digest: string,
    #[field(string)]
    updated: string,
    #[relation(Strand, many2one, root)]
    strand: Strand,
    #[relation(StrandInbox, one2one, opt)]
    inbox: StrandInbox,
}

#[resource]
pub(crate) struct InboxReceipt {
    #[field(string, unique)]
    tag: string,
    #[field(
        string,
        default = "accepted",
        values = ("accepted", "completed", "driving", "failed", "recovered")
    )]
    state: string,
    #[field(string)]
    accepted: string,
    #[field(string)]
    updated: string,
    #[relation(Strand, many2one, root)]
    strand: Strand,
}

#[resource(frozen)]
pub(crate) struct ReceiptTransition {
    #[field(string, unique)]
    tag: string,
    #[field(int, unique = receipt, min = 1)]
    sequence: int,
    #[field(
        string,
        values = ("accepted", "completed", "driving", "failed", "recovered")
    )]
    state: string,
    #[field(string, opt)]
    incident: string,
    #[field(string, opt)]
    rebuilt: string,
    #[field(string)]
    occurred: string,
    #[relation(InboxReceipt, many2one, root)]
    receipt: InboxReceipt,
    #[relation(Turn, many2one, opt)]
    turn: Turn,
}
