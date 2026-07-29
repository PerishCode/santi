use keel::Graph;

mod edge;
mod history;
mod inbox;
mod job;
mod ledger;
mod ops;

pub(super) fn graph() -> Graph {
    let mut graph = Graph::new();
    graph
        .plug::<ledger::Soul>()
        .plug::<ledger::Strand>()
        .plug::<ledger::Turn>()
        .plug::<ledger::TurnCompletion>()
        .plug::<ledger::TurnFailure>()
        .plug::<ledger::Message>()
        .plug::<ledger::StrandEntry>()
        .plug::<ledger::TurnStop>()
        .plug::<job::ToolCall>()
        .plug::<job::StrandEffect>()
        .plug::<job::Job>()
        .plug::<inbox::StrandInbox>()
        .plug::<inbox::InboxSlot>()
        .plug::<inbox::InboxReceipt>()
        .plug::<inbox::ReceiptTransition>()
        .plug::<history::MessageEvent>()
        .plug::<history::ToolResult>()
        .plug::<history::ToolOutput>()
        .plug::<history::ToolFailure>()
        .plug::<history::ThinkingSpan>()
        .plug::<history::ThinkingCompletion>()
        .plug::<history::ThinkingFailure>()
        .plug::<history::Compact>()
        .plug::<edge::Webhook>()
        .plug::<edge::WebhookDelivery>()
        .plug::<edge::Downstream>()
        .plug::<edge::DownstreamIngest>()
        .plug::<edge::Environ>()
        .plug::<ops::JobCapability>()
        .plug::<ops::TraceRecord>()
        .plug::<ops::OutboxStream>()
        .plug::<ops::TurnOutbox>()
        .plug::<ops::error::ErrorIncident>()
        .plug::<ops::error::ResolvedIncident>()
        .plug::<ops::error::ErrorTransition>()
        .plug::<ops::error::ErrorAcknowledgement>();
    graph
}
