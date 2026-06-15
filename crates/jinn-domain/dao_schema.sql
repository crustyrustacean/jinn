-- Post-v20 final shape (the state sessions.db reaches after migrate_v20).
-- The #[dao] macro validates #[query]/#[execute] SQL against THIS schema.
--
-- This mirrors the authoritative columns written by `NewSessionRow` in the
-- pre-port sqlite.rs. The six "zombie" columns (profile, blobs, cwd,
-- lifecycle_name, lifecycle_args, lifecycle_script_state) AND the vestigial
-- judge_meta column are dropped by v20 — none are ever written or read by
-- application code (judge_meta is referenced only by an orphaned doc comment
-- in session_store.rs). Result: exactly 9 authoritative columns.

CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    title TEXT,
    updated_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    parent_session TEXT DEFAULT NULL,
    archived BOOLEAN NOT NULL DEFAULT FALSE,
    metadata TEXT,
    is_automated BOOLEAN NOT NULL DEFAULT FALSE,
    persist BOOLEAN NOT NULL DEFAULT TRUE
);

CREATE TABLE entries (
    id TEXT PRIMARY KEY,
    timing TEXT NOT NULL,
    kind TEXT NOT NULL,
    context_history TEXT NOT NULL DEFAULT '[]'
);

CREATE TABLE session_history (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    entry_id TEXT NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL,
    pin_position TEXT DEFAULT NULL,
    ignored BOOLEAN NOT NULL DEFAULT FALSE,
    context_override TEXT NOT NULL DEFAULT 'default',
    PRIMARY KEY (session_id, entry_id),
    UNIQUE (session_id, ordinal)
);

CREATE TABLE token_ledger (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    timestamp TEXT NOT NULL,
    tokens_sent INTEGER NOT NULL,
    tokens_received INTEGER NOT NULL,
    cost DOUBLE,
    model_used TEXT
);
