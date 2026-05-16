//! Session database migrations.
//!
//! Defines the ordered list of migrations for the session SQLite database.
//! The initial migration (v0) creates the baseline schema that previously
//! lived in the `SCHEMA` const in `sqlite.rs`.

use error_stack::{Report, ResultExt as _};
use rusqlite::Connection;

use crate::common::migration::{Migration, MigrationError};

/// Creates the initial session database schema.
///
/// This is the baseline migration — it creates the same tables that were
/// previously defined in the `SCHEMA` const. Uses `CREATE TABLE IF NOT EXISTS`
/// so it is idempotent on databases that already have these tables.
fn v0_create_initial_schema(conn: &Connection) -> Result<(), Report<MigrationError>> {
    conn.execute_batch(
        "
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
        ",
    )
    .change_context(MigrationError)
    .attach("failed to create initial schema")?;

    Ok(())
}

/// Adds the `cwd` column to the `sessions` table.
///
/// Sessions created before this migration have no cwd, so the default is `.`
/// which resolves to the current directory on load. The session actor's
/// CWD validation will handle the fallback to the global cwd if needed.
fn v1_add_cwd_column(conn: &Connection) -> Result<(), Report<MigrationError>> {
    conn.execute_batch(
        "ALTER TABLE sessions ADD COLUMN cwd TEXT NOT NULL DEFAULT '.';",
    )
    .change_context(MigrationError)
    .attach("failed to add cwd column")?;

    Ok(())
}

/// Returns the ordered list of session database migrations.
pub fn session_migrations() -> Vec<Migration> {
    vec![
        Migration {
            version: 0,
            name: "create_initial_schema",
            up: v0_create_initial_schema,
        },
        Migration {
            version: 1,
            name: "add_cwd_column",
            up: v1_add_cwd_column,
        },
    ]
}
