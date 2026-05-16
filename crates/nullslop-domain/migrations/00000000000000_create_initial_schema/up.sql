CREATE TABLE IF NOT EXISTS sessions (
    id               TEXT PRIMARY KEY,
    title            TEXT,
    updated_at       TEXT NOT NULL,
    profile          TEXT NOT NULL DEFAULT '{}',
    strategy_state   TEXT NOT NULL DEFAULT '{}',
    blobs            TEXT NOT NULL DEFAULT '{}',
    parent_session   TEXT DEFAULT NULL
);

CREATE TABLE IF NOT EXISTS entries (
    id         TEXT PRIMARY KEY,
    timestamp  TEXT NOT NULL,
    kind       TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS session_entries (
    session_id    TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    entry_id      TEXT NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
    ordinal       INTEGER NOT NULL,
    pin_position  TEXT DEFAULT NULL,
    PRIMARY KEY (session_id, entry_id),
    UNIQUE (session_id, ordinal)
);

CREATE TABLE IF NOT EXISTS token_ledger (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id       TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    timestamp        TEXT NOT NULL,
    tokens_sent      INTEGER NOT NULL,
    tokens_received  INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_session_entries_session
    ON session_entries(session_id, ordinal);

CREATE INDEX IF NOT EXISTS idx_token_ledger_session
    ON token_ledger(session_id);
