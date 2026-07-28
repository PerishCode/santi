use santi_model::ingest;

#[derive(Clone, Copy)]
pub struct WebhookDraft<'a> {
    pub name: &'a str,
    pub adaptor: &'a str,
    pub soul: &'a str,
    pub strategy: &'a str,
    pub credential: &'a str,
    pub created: &'a str,
}

#[derive(Clone, Copy)]
pub struct DownstreamDraft<'a> {
    pub tag: &'a str,
    pub prefix: &'a str,
    pub digest: &'a str,
    pub created: &'a str,
}

#[derive(Clone, Copy)]
pub enum ReplayDraft<'a> {
    Webhook {
        subscription: &'a str,
        delivery: &'a str,
        digest: &'a str,
    },
    Downstream {
        owner: &'a str,
        request: &'a str,
        digest: &'a str,
    },
}

#[derive(Debug, Clone)]
pub struct Accepted {
    pub receipt: ingest::Receipt,
    pub inserted: bool,
}
