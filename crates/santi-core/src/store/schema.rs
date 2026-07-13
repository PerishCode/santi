pub(super) const SCHEMA: &str = r#"
-- A soul is id-only: its identity is its memory (a file, rendered live into
-- [santi-soul]), never a profile row. Timestamps are pure provenance.
CREATE TABLE IF NOT EXISTS souls (
    id TEXT PRIMARY KEY,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS webhooks (
    name TEXT PRIMARY KEY,
    adaptor TEXT NOT NULL,
    soul_id TEXT NOT NULL,
    strand_strategy TEXT NOT NULL,
    secret_env TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    actor_type TEXT NOT NULL CHECK (actor_type IN ('soul', 'system')),
    actor_id TEXT NOT NULL,
    message_kind TEXT NOT NULL DEFAULT 'text' CHECK (message_kind IN ('text', 'santi_system')),
    content TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('pending', 'fixed', 'aborted')),
    version INTEGER NOT NULL DEFAULT 1 CHECK (version > 0),
    is_request INTEGER NOT NULL DEFAULT 0 CHECK (is_request IN (0, 1)),
    deleted_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS message_events (
    id TEXT PRIMARY KEY,
    message_id TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('patch', 'insert', 'remove', 'fix', 'delete')),
    actor_type TEXT NOT NULL CHECK (actor_type IN ('soul', 'system')),
    actor_id TEXT NOT NULL,
    base_version INTEGER NOT NULL CHECK (base_version > 0),
    payload TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS strand_effects (
    id TEXT PRIMARY KEY,
    strand_id TEXT NOT NULL,
    turn_id TEXT NOT NULL,
    tool_call_id TEXT UNIQUE,
    effect_type TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN (
        'prepared', 'dispatching', 'unknown', 'confirmed', 'not_dispatched',
        'resolved_applied', 'resolved_not_applied'
    )),
    result_ref TEXT,
    error_text TEXT,
    metadata TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    dispatched_at TEXT,
    settled_at TEXT
);

CREATE TABLE IF NOT EXISTS effect_transitions (
    id TEXT PRIMARY KEY,
    effect_id TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    state TEXT NOT NULL CHECK (state IN (
        'prepared', 'dispatching', 'unknown', 'confirmed', 'not_dispatched',
        'resolved_applied', 'resolved_not_applied'
    )),
    reason TEXT NOT NULL,
    evidence TEXT,
    occurred_at TEXT NOT NULL,
    UNIQUE (effect_id, sequence)
);

CREATE TABLE IF NOT EXISTS strands (
    id TEXT PRIMARY KEY,
    soul_id TEXT NOT NULL,
    external_label TEXT,
    strand_memory TEXT NOT NULL DEFAULT '',
    provider_state TEXT,
    next_seq INTEGER NOT NULL DEFAULT 1 CHECK (next_seq > 0),
    last_seen_strand_seq INTEGER NOT NULL DEFAULT 0 CHECK (last_seen_strand_seq >= 0),
    parent_strand_id TEXT,
    fork_point INTEGER,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_strands_external_label ON strands (soul_id, external_label) WHERE external_label IS NOT NULL;

CREATE TABLE IF NOT EXISTS turns (
    id TEXT PRIMARY KEY,
    strand_id TEXT NOT NULL,
    trigger_type TEXT NOT NULL CHECK (trigger_type IN ('strand_send', 'system')),
    trigger_ref TEXT,
    base_strand_seq INTEGER NOT NULL CHECK (base_strand_seq >= 0),
    end_strand_seq INTEGER CHECK (end_strand_seq IS NULL OR end_strand_seq >= 0),
    status TEXT NOT NULL CHECK (status IN ('running', 'completed', 'failed')),
    error_text TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    finished_at TEXT
);

-- Neutral occurrence: what tool was called with what arguments. Provider-
-- agnostic, part of the durable-occurrence timeline. It carries NO provider
-- wire plumbing (see provider_replay_material) — a neutral read (soul context,
-- audit, fork) structurally cannot reach provider-specific ids/blobs.
CREATE TABLE IF NOT EXISTS tool_calls (
    id TEXT PRIMARY KEY,
    turn_id TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    arguments TEXT NOT NULL,
    created_at TEXT NOT NULL
);

-- Adaptor-owned, provider-scoped REPLAY MATERIAL for a tool_call: the raw wire
-- item + wire ids the adaptor used, kept ONLY so a provider adaptor can re-present
-- the call to that same provider. It is NOT occurrence truth and only the adaptor
-- projection may interpret it (PHASE-09 decision #9). `kind`:
--   'regenerable'   — advisory cache; if invalid, drop it and synthesize from the
--                     neutral fields (the fc-id poison heals here).
--   'irreplaceable' — a credential/state that cannot be re-synthesized (e.g. a
--                     future encrypted-reasoning blob); on invalid/missing it is
--                     quarantined/omitted, NEVER silently regenerated.
CREATE TABLE IF NOT EXISTS provider_replay_material (
    tool_call_id TEXT PRIMARY KEY,
    provider_family TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('regenerable', 'irreplaceable')),
    blob TEXT,
    item_id TEXT,
    response_id TEXT,
    schema_version INTEGER,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS tool_results (
    id TEXT PRIMARY KEY,
    tool_call_id TEXT NOT NULL,
    output TEXT,
    error_text TEXT,
    created_at TEXT NOT NULL,
    UNIQUE (tool_call_id),
    CHECK (
        (output IS NOT NULL AND error_text IS NULL) OR
        (output IS NULL AND error_text IS NOT NULL)
    )
);

CREATE TABLE IF NOT EXISTS thinking_spans (
    id TEXT PRIMARY KEY,
    turn_id TEXT NOT NULL,
    provider_response_id TEXT,
    state TEXT NOT NULL CHECK (state IN ('running', 'completed', 'failed')),
    summary TEXT,
    completion_reason TEXT CHECK (
        completion_reason IS NULL OR
        completion_reason IN ('first_text_delta', 'tool_call_requested', 'provider_completed')
    ),
    error_text TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    finished_at TEXT,
    CHECK (
        (state = 'failed' AND error_text IS NOT NULL) OR
        (state <> 'failed' AND error_text IS NULL)
    )
);

CREATE TABLE IF NOT EXISTS compacts (
    id TEXT PRIMARY KEY,
    strand_id TEXT NOT NULL,
    summary TEXT NOT NULL,
    start_message_id TEXT NOT NULL,
    end_message_id TEXT NOT NULL,
    created_at TEXT,
    metadata TEXT
);
CREATE INDEX IF NOT EXISTS idx_compacts_strand ON compacts (strand_id);

CREATE TABLE IF NOT EXISTS error_incidents (
    id TEXT PRIMARY KEY,
    incident_key TEXT NOT NULL,
    code TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('active', 'resolved')),
    category TEXT NOT NULL,
    severity TEXT NOT NULL,
    retry TEXT NOT NULL,
    exposure TEXT NOT NULL,
    scope_kind TEXT NOT NULL,
    scope_id TEXT NOT NULL,
    source_component TEXT NOT NULL,
    source_operation TEXT NOT NULL,
    latest_source_component TEXT NOT NULL,
    latest_source_operation TEXT NOT NULL,
    message TEXT NOT NULL,
    latest_message TEXT NOT NULL,
    context TEXT NOT NULL,
    latest_context TEXT NOT NULL,
    occurrence_count INTEGER NOT NULL CHECK (occurrence_count > 0),
    revision INTEGER NOT NULL CHECK (revision > 0),
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    resolved_at TEXT,
    resolved_by TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_error_incidents_active_key
ON error_incidents(incident_key)
WHERE status = 'active';
CREATE INDEX IF NOT EXISTS idx_error_incidents_scope_time
ON error_incidents(scope_kind, scope_id, first_seen_at);

CREATE TABLE IF NOT EXISTS error_transitions (
    id TEXT PRIMARY KEY,
    incident_id TEXT NOT NULL,
    revision INTEGER NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('opened', 'resolved')),
    payload TEXT NOT NULL,
    created_at TEXT NOT NULL,
    delivered_at TEXT,
    UNIQUE (incident_id, revision)
);
CREATE INDEX IF NOT EXISTS idx_error_transitions_pending
ON error_transitions(created_at, id)
WHERE delivered_at IS NULL;

CREATE TABLE IF NOT EXISTS strand_inbox (
    id TEXT PRIMARY KEY,
    strand_id TEXT NOT NULL,
    message_kind TEXT NOT NULL CHECK (message_kind IN ('text', 'santi_system')),
    content TEXT NOT NULL,
    source_type TEXT,
    source_ref TEXT,
    source_metadata TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_strand_inbox_strand_created_at ON strand_inbox (strand_id, created_at);

-- Durable responsibility root for every accepted inbox item. Content remains
-- in the inbox/timeline; this table carries only obligation state and locators.
CREATE TABLE IF NOT EXISTS inbox_receipts (
    id TEXT PRIMARY KEY,
    strand_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN (
        'accepted', 'mechanically_recovered', 'driving', 'turn_failed', 'completed'
    )),
    accepted_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_inbox_receipts_strand_state
ON inbox_receipts(strand_id, state, accepted_at);

-- Minimal state evidence. Turn and incident details remain canonical in their
-- own tables; transitions retain only the locators needed to explain a receipt.
CREATE TABLE IF NOT EXISTS receipt_transitions (
    id TEXT PRIMARY KEY,
    inbox_id TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    state TEXT NOT NULL CHECK (state IN (
        'accepted', 'mechanically_recovered', 'driving', 'turn_failed', 'completed'
    )),
    turn_id TEXT,
    incident_id TEXT,
    reconstructed_from TEXT,
    occurred_at TEXT NOT NULL,
    UNIQUE (inbox_id, sequence)
);
CREATE INDEX IF NOT EXISTS idx_receipt_transitions_receipt_time
ON receipt_transitions(inbox_id, sequence);

CREATE TABLE IF NOT EXISTS r_strand_entries (
    strand_id TEXT NOT NULL,
    target_type TEXT NOT NULL CHECK (target_type IN ('message', 'thinking', 'tool_call', 'tool_result')),
    target_id TEXT NOT NULL,
    strand_seq INTEGER NOT NULL CHECK (strand_seq > 0),
    created_at TEXT NOT NULL,
    PRIMARY KEY (strand_id, target_type, target_id),
    UNIQUE (strand_id, strand_seq)
);

CREATE INDEX IF NOT EXISTS idx_messages_actor_created_at ON messages (actor_type, actor_id, created_at);
CREATE INDEX IF NOT EXISTS idx_messages_state_created_at ON messages (state, created_at);
CREATE INDEX IF NOT EXISTS idx_message_events_message_id_created_at ON message_events (message_id, created_at);
CREATE INDEX IF NOT EXISTS idx_strand_effects_strand_created_at ON strand_effects (strand_id, created_at);
CREATE INDEX IF NOT EXISTS idx_strand_effects_turn_created_at ON strand_effects (turn_id, created_at);
CREATE INDEX IF NOT EXISTS idx_strand_effects_state_updated_at ON strand_effects (state, updated_at);
CREATE INDEX IF NOT EXISTS idx_effect_transitions_effect_sequence ON effect_transitions (effect_id, sequence);
CREATE INDEX IF NOT EXISTS idx_strands_soul_id ON strands (soul_id);
CREATE INDEX IF NOT EXISTS idx_strands_lineage ON strands (parent_strand_id, fork_point);
CREATE INDEX IF NOT EXISTS idx_turns_strand_created_at ON turns (strand_id, created_at);
CREATE INDEX IF NOT EXISTS idx_turns_strand_status_created_at ON turns (strand_id, status, created_at);
CREATE INDEX IF NOT EXISTS idx_tool_calls_turn_id_created_at ON tool_calls (turn_id, created_at);
CREATE INDEX IF NOT EXISTS idx_tool_results_tool_call_id ON tool_results (tool_call_id);
CREATE INDEX IF NOT EXISTS idx_thinking_spans_turn_id_created_at ON thinking_spans (turn_id, created_at);
CREATE INDEX IF NOT EXISTS idx_r_strand_entries_target_lookup ON r_strand_entries (target_type, target_id);
CREATE INDEX IF NOT EXISTS idx_r_strand_entries_seq ON r_strand_entries (strand_id, strand_seq);

-- ── IM layer (im_*) ──────────────────────────────────────────────────────────
-- A plain messenger integrated into the santi binary for cold-start; conceptually
-- ORTHOGONAL to the runtime (souls/strands/turns). These tables are the IM's own
-- store — the runtime never reads them. A participant is a persistent messaging
-- endpoint (a human/CLI peer with a passive inbox; a soul participant's "inbox" is
-- its strand and is NOT stored here). Reply-routing authority lives entirely
-- here, in the IM's envelope. The runtime inbox may carry bounded diagnostic
-- source provenance, but that is not a reply capability or provider-visible
-- message content.
CREATE TABLE IF NOT EXISTS im_participants (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('human', 'soul')),
    created_at TEXT NOT NULL
);

-- The passive inbox for a (human/CLI) participant: the return values it catches.
-- `seq` is a global monotonic cursor (caller polls `WHERE participant_id=? AND
-- seq > since`); `from_ref` names the soul strand that replied. Retained for audit
-- (the IM conversation history); no ack — the caller's high-water `seq` is the ack.
CREATE TABLE IF NOT EXISTS im_inbox (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    participant_id TEXT NOT NULL,
    from_ref TEXT,
    content TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_im_inbox_participant_seq ON im_inbox (participant_id, seq);
"#;
