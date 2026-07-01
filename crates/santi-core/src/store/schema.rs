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
    session_strategy TEXT NOT NULL,
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

CREATE TABLE IF NOT EXISTS session_effects (
    id TEXT PRIMARY KEY,
    strand_id TEXT NOT NULL,
    effect_type TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    status TEXT NOT NULL,
    source_hook_id TEXT NOT NULL,
    source_turn_id TEXT NOT NULL,
    result_ref TEXT,
    error_text TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (strand_id, effect_type, idempotency_key)
);

CREATE TABLE IF NOT EXISTS strands (
    id TEXT PRIMARY KEY,
    soul_id TEXT NOT NULL,
    external_label TEXT,
    session_memory TEXT NOT NULL DEFAULT '',
    provider_state TEXT,
    next_seq INTEGER NOT NULL DEFAULT 1 CHECK (next_seq > 0),
    last_seen_session_seq INTEGER NOT NULL DEFAULT 0 CHECK (last_seen_session_seq >= 0),
    parent_strand_id TEXT,
    fork_point INTEGER,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_strands_external_label ON strands (soul_id, external_label) WHERE external_label IS NOT NULL;

CREATE TABLE IF NOT EXISTS turns (
    id TEXT PRIMARY KEY,
    strand_id TEXT NOT NULL,
    trigger_type TEXT NOT NULL CHECK (trigger_type IN ('session_send', 'system')),
    trigger_ref TEXT,
    base_strand_seq INTEGER NOT NULL CHECK (base_strand_seq >= 0),
    end_strand_seq INTEGER CHECK (end_strand_seq IS NULL OR end_strand_seq >= 0),
    status TEXT NOT NULL CHECK (status IN ('running', 'completed', 'failed')),
    error_text TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    finished_at TEXT
);

CREATE TABLE IF NOT EXISTS tool_calls (
    id TEXT PRIMARY KEY,
    turn_id TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    arguments TEXT NOT NULL,
    provider_item TEXT,
    item_id TEXT,
    response_id TEXT,
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
    end_message_id TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_compacts_strand ON compacts (strand_id);

CREATE TABLE IF NOT EXISTS strand_inbox (
    id TEXT PRIMARY KEY,
    strand_id TEXT NOT NULL,
    message_kind TEXT NOT NULL CHECK (message_kind IN ('text', 'santi_system')),
    content TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_strand_inbox_strand_created_at ON strand_inbox (strand_id, created_at);

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
CREATE INDEX IF NOT EXISTS idx_session_effects_strand_created_at ON session_effects (strand_id, created_at);
CREATE INDEX IF NOT EXISTS idx_session_effects_lookup ON session_effects (strand_id, effect_type, idempotency_key);
CREATE INDEX IF NOT EXISTS idx_strands_soul_id ON strands (soul_id);
CREATE INDEX IF NOT EXISTS idx_strands_lineage ON strands (parent_strand_id, fork_point);
CREATE INDEX IF NOT EXISTS idx_turns_strand_created_at ON turns (strand_id, created_at);
CREATE INDEX IF NOT EXISTS idx_turns_strand_status_created_at ON turns (strand_id, status, created_at);
CREATE INDEX IF NOT EXISTS idx_tool_calls_turn_id_created_at ON tool_calls (turn_id, created_at);
CREATE INDEX IF NOT EXISTS idx_tool_results_tool_call_id ON tool_results (tool_call_id);
CREATE INDEX IF NOT EXISTS idx_thinking_spans_turn_id_created_at ON thinking_spans (turn_id, created_at);
CREATE INDEX IF NOT EXISTS idx_r_strand_entries_target_lookup ON r_strand_entries (target_type, target_id);
CREATE INDEX IF NOT EXISTS idx_r_strand_entries_seq ON r_strand_entries (strand_id, strand_seq);
"#;
