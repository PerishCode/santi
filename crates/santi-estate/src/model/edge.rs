use super::ledger::Soul;
use keel::atom::string;
use keel::resource;

#[resource]
pub(crate) struct Webhook {
    #[field(string, unique)]
    name: string,
    #[field(string)]
    adaptor: string,
    #[field(string)]
    strategy: string,
    #[field(string)]
    credential: string,
    #[field(string)]
    created: string,
    #[field(string)]
    updated: string,
    #[relation(Soul, many2one, root)]
    soul: Soul,
}

#[resource(frozen)]
pub(crate) struct WebhookDelivery {
    #[field(string, unique = webhook)]
    delivery: string,
    #[field(string)]
    request_sha256: string,
    #[field(string)]
    created: string,
    #[relation(Webhook, many2one, root)]
    webhook: Webhook,
    #[field(string)]
    strand: string,
    #[field(string)]
    inbox: string,
}

#[resource]
pub(crate) struct Downstream {
    #[field(string, unique)]
    tag: string,
    #[field(string, unique)]
    prefix: string,
    #[field(string, unique)]
    credential_sha256: string,
    #[field(string)]
    created: string,
    #[field(string)]
    updated: string,
}

#[resource(frozen)]
pub(crate) struct DownstreamIngest {
    #[field(string, unique = downstream)]
    request: string,
    #[field(string)]
    request_sha256: string,
    #[field(string)]
    created: string,
    #[relation(Downstream, many2one, root)]
    downstream: Downstream,
    #[field(string)]
    strand: string,
    #[field(string)]
    inbox: string,
}
