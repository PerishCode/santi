pub const COMPONENT_DESCRIPTIONS: [(&str, &str); 10] = [
    (
        "Soul",
        "A soul is a cyber-individual, keyed by id alone. It has no name/avatar/desc\ncolumn: identity is the mutable self, and it lives entirely in the soul's\nmemory (rendered live into `[santi-soul]`), not in a profile row. The\ntimestamps are pure provenance.",
    ),
    (
        "soul::Draft",
        "Create a new soul (an individual). Souls are API-managed, never config.\nA soul is id-only; its identity is its memory, so the only thing to supply\nat creation is the initial `[santi-soul]` memory to seed (empty/absent → a\nblank soul that will author its own).",
    ),
    (
        "webhook::Subscription",
        "An API-managed webhook subscription: how an external source reaches a soul.\n`adaptor` selects the boundary normalizer (integration knowledge); `soul`\nis who receives the resulting turn; `strategy` picks where the thread\nlives (`per_thread` = one strand per adaptor-derived label, `single` = one\nstrand per subscription); `credential` names the env var holding the signing\nsecret (the secret itself is never stored). The `name` is the URL path segment.",
    ),
    (
        "webhook::Draft",
        "The complete desired webhook subscription. POSTing it is an idempotent\nensure: an absent name is created, an identical subscription is returned\nunchanged, and drift under an existing name is rejected with 409.",
    ),
    (
        "receipt::State",
        "Current durable responsibility state for one accepted inbox item. A\nmechanically-recovered transition can be immediately followed by `driving`\nin the same transaction; callers inspect `transitions` for that evidence.",
    ),
    (
        "Compact",
        "A compact is a pure projection overlay over a strand's spine. It\nself-describes its coverage by message-id boundaries and carries the\noperator-authored summary while originals remain queryable.",
    ),
    (
        "effect::State",
        "Durable truth for one concrete external-effect attempt. It is deliberately\nnot turn state: one turn may contain several independently settled or\nambiguous effects.",
    ),
    (
        "job::Job",
        "A soul-owned detached command with an execution lifecycle independent of\nits creating shell invocation, tool call, effect, turn, and receipt. Origin\nfields are immutable provenance and never imply cascading state.",
    ),
    (
        "job::State",
        "`accepted` is the durable create success boundary, not proof that the\ncommand started. `unknown` means retained evidence cannot prove either a live\nmatching supervisor generation or a terminal outcome; Santi never replays it\nautomatically.",
    ),
    (
        "message::Role",
        "No user/account actor: santi is individual-first, not multi-tenant. All\ninbound (a CLI send, a webhook event) arrives as `System` — the sender's\nidentity is metainfo carried in the content, opaque to core, not a distinct\nactor kind. `(actor, kind)` is the full marker at the provider\nboundary (see `item`): Soul→assistant, System+Text→user\n(world-inbound), System+SantiSystem→system (runtime-meta, not user speech).",
    ),
];
