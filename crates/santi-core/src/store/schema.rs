pub(super) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS accounts (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS souls (
    id TEXT PRIMARY KEY,
    memory TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS soul_profiles (
    soul_id TEXT PRIMARY KEY,
    soul_name TEXT NOT NULL,
    nickname TEXT NOT NULL,
    avatar_ref TEXT,
    avatar_seed TEXT NOT NULL,
    desc TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    parent_session_id TEXT,
    fork_point INTEGER,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS session_profiles (
    session_id TEXT PRIMARY KEY,
    title TEXT,
    desc TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    actor_type TEXT NOT NULL CHECK (actor_type IN ('account', 'soul', 'system')),
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

CREATE TABLE IF NOT EXISTS r_session_messages (
    session_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    session_seq INTEGER NOT NULL CHECK (session_seq > 0),
    created_at TEXT NOT NULL,
    PRIMARY KEY (session_id, message_id),
    UNIQUE (session_id, session_seq)
);

CREATE TABLE IF NOT EXISTS message_events (
    id TEXT PRIMARY KEY,
    message_id TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('patch', 'insert', 'remove', 'fix', 'delete')),
    actor_type TEXT NOT NULL CHECK (actor_type IN ('account', 'soul', 'system')),
    actor_id TEXT NOT NULL,
    base_version INTEGER NOT NULL CHECK (base_version > 0),
    payload TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS session_effects (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    effect_type TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    status TEXT NOT NULL,
    source_hook_id TEXT NOT NULL,
    source_turn_id TEXT NOT NULL,
    result_ref TEXT,
    error_text TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (session_id, effect_type, idempotency_key)
);

CREATE TABLE IF NOT EXISTS soul_sessions (
    id TEXT PRIMARY KEY,
    soul_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    session_memory TEXT NOT NULL DEFAULT '',
    provider_state TEXT,
    next_seq INTEGER NOT NULL DEFAULT 1 CHECK (next_seq > 0),
    last_seen_session_seq INTEGER NOT NULL DEFAULT 0 CHECK (last_seen_session_seq >= 0),
    parent_soul_session_id TEXT,
    fork_point INTEGER,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (soul_id, session_id)
);

CREATE TABLE IF NOT EXISTS turns (
    id TEXT PRIMARY KEY,
    soul_session_id TEXT NOT NULL,
    trigger_type TEXT NOT NULL CHECK (trigger_type IN ('session_send', 'system')),
    trigger_ref TEXT,
    input_through_session_seq INTEGER NOT NULL CHECK (input_through_session_seq >= 0),
    base_soul_session_seq INTEGER NOT NULL CHECK (base_soul_session_seq >= 0),
    end_soul_session_seq INTEGER CHECK (end_soul_session_seq IS NULL OR end_soul_session_seq >= 0),
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
    turn_id TEXT NOT NULL,
    summary TEXT NOT NULL,
    start_session_seq INTEGER NOT NULL CHECK (start_session_seq > 0),
    end_session_seq INTEGER NOT NULL CHECK (end_session_seq > 0),
    created_at TEXT NOT NULL,
    CHECK (start_session_seq <= end_session_seq)
);

CREATE TABLE IF NOT EXISTS r_soul_session_messages (
    soul_session_id TEXT NOT NULL,
    target_type TEXT NOT NULL CHECK (target_type IN ('message', 'compact', 'thinking', 'tool_call', 'tool_result')),
    target_id TEXT NOT NULL,
    soul_session_seq INTEGER NOT NULL CHECK (soul_session_seq > 0),
    created_at TEXT NOT NULL,
    PRIMARY KEY (soul_session_id, target_type, target_id),
    UNIQUE (soul_session_id, soul_session_seq)
);

CREATE INDEX IF NOT EXISTS idx_messages_actor_created_at ON messages (actor_type, actor_id, created_at);
CREATE INDEX IF NOT EXISTS idx_messages_state_created_at ON messages (state, created_at);
CREATE INDEX IF NOT EXISTS idx_session_profiles_title ON session_profiles (title);
CREATE INDEX IF NOT EXISTS idx_r_session_messages_message_id ON r_session_messages (message_id);
CREATE INDEX IF NOT EXISTS idx_r_session_messages_session_seq ON r_session_messages (session_id, session_seq);
CREATE INDEX IF NOT EXISTS idx_message_events_message_id_created_at ON message_events (message_id, created_at);
CREATE INDEX IF NOT EXISTS idx_session_effects_session_created_at ON session_effects (session_id, created_at);
CREATE INDEX IF NOT EXISTS idx_session_effects_lookup ON session_effects (session_id, effect_type, idempotency_key);
CREATE INDEX IF NOT EXISTS idx_sessions_lineage ON sessions (parent_session_id, fork_point);
CREATE INDEX IF NOT EXISTS idx_soul_sessions_session_id ON soul_sessions (session_id);
CREATE INDEX IF NOT EXISTS idx_soul_sessions_soul_id ON soul_sessions (soul_id);
CREATE INDEX IF NOT EXISTS idx_soul_sessions_lineage ON soul_sessions (parent_soul_session_id, fork_point);
CREATE INDEX IF NOT EXISTS idx_turns_soul_session_created_at ON turns (soul_session_id, created_at);
CREATE INDEX IF NOT EXISTS idx_turns_soul_session_status_created_at ON turns (soul_session_id, status, created_at);
CREATE INDEX IF NOT EXISTS idx_tool_calls_turn_id_created_at ON tool_calls (turn_id, created_at);
CREATE INDEX IF NOT EXISTS idx_tool_results_tool_call_id ON tool_results (tool_call_id);
CREATE INDEX IF NOT EXISTS idx_thinking_spans_turn_id_created_at ON thinking_spans (turn_id, created_at);
CREATE INDEX IF NOT EXISTS idx_compacts_turn_id_created_at ON compacts (turn_id, created_at);
CREATE INDEX IF NOT EXISTS idx_r_soul_session_messages_target_lookup ON r_soul_session_messages (target_type, target_id);
CREATE INDEX IF NOT EXISTS idx_r_soul_session_messages_seq ON r_soul_session_messages (soul_session_id, soul_session_seq);
"#;
