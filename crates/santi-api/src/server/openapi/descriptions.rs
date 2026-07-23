pub const COMPONENT_DESCRIPTIONS: [(&str, &str); 7] = [
    (
        "Soul",
        "A soul is a cyber-individual, keyed by id alone. It has no name/avatar/desc\ncolumn: identity is the mutable self, and it lives entirely in the soul's\nmemory (rendered live into `[santi-soul]`), not in a profile row. The\ntimestamps are pure provenance.",
    ),
    (
        "CreateSoulRequest",
        "Create a new soul (an individual). Souls are API-managed, never config.\nA soul is id-only; its identity is its memory, so the only thing to supply\nat creation is the initial `[santi-soul]` memory to seed (empty/absent → a\nblank soul that will author its own).",
    ),
    (
        "WebhookSubscription",
        "An API-managed webhook subscription: how an external source reaches a soul.\n`adaptor` selects the boundary normalizer (integration knowledge); `soul`\nis who receives the resulting turn; `strategy` picks where the thread\nlives (`per_thread` = one strand per adaptor-derived label, `single` = one\nstrand per subscription); `credential` names the env var holding the signing\nsecret (the secret itself is never stored). The `name` is the URL path segment.",
    ),
    (
        "ReceiptState",
        "Current durable responsibility state for one accepted inbox item. A\nmechanically-recovered transition can be immediately followed by `driving`\nin the same transaction; callers inspect `transitions` for that evidence.",
    ),
    (
        "Compact",
        "A compact is a pure projection overlay over a strand's spine. It\nself-describes its coverage by message-id boundaries and carries the\noperator-authored summary while originals remain queryable.",
    ),
    (
        "EffectState",
        "Durable truth for one concrete external-effect attempt. It is deliberately\nnot turn state: one turn may contain several independently settled or\nambiguous effects.",
    ),
    (
        "ActorType",
        "No user/account actor: santi is individual-first, not multi-tenant. All\ninbound (a CLI send, a webhook event) arrives as `System` — the sender's\nidentity is metainfo carried in the content, opaque to core, not a distinct\nactor kind. `(actor, kind)` is the full marker at the provider\nboundary (see `message_to_provider_item`): Soul→assistant, System+Text→user\n(world-inbound), System+SantiSystem→system (runtime-meta, not user speech).",
    ),
];
