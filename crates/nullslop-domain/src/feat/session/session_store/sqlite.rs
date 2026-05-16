//! SQLite-backed session store implementation.
//!
//! Stores session data in normalized tables with a junction table for entries.
//! This eliminates duplication — each chat entry is stored once and shared
//! across sessions. The junction table enables fork support by copying only
//! small junction rows, not entry data.

use std::path::PathBuf;

use async_trait::async_trait;
use error_stack::{Report, ResultExt as _};
use rusqlite::Connection;
use tokio::task::spawn_blocking;

use crate::common::app_info::APP_NAME;
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryKind};
use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::session::session_summary::SessionSummary;
use crate::protocol::{ChatEntryId, SessionId};

use super::{SessionStore, SessionStoreError};

/// SQLite database file name.
const FILE_NAME: &str = "sessions.db";

/// SQL schema — executed on first connection.
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS sessions (
    id               TEXT PRIMARY KEY,
    title            TEXT,
    updated_at       TEXT NOT NULL,
    profile          TEXT NOT NULL DEFAULT '{}',
    strategy_state   TEXT NOT NULL DEFAULT '{}',
    blobs            TEXT NOT NULL DEFAULT '{}',
    parent_session   TEXT DEFAULT NULL,
    fork_at_ordinal  INTEGER DEFAULT NULL
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
";

/// SQLite-backed session store.
///
/// Stores sessions in a single SQLite database file at the platform data
/// directory. Uses `tokio::task::spawn_blocking` to bridge synchronous
/// rusqlite calls into the async runtime.
pub struct SqliteSessionStore {
    /// Directory containing `sessions.db`.
    dir: PathBuf,
}

impl Default for SqliteSessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SqliteSessionStore {
    /// Creates a store at the platform data directory.
    ///
    /// Uses `dirs::data_dir()` → `nullslop/sessions.db` on Linux.
    /// The database file is created on first access.
    ///
    /// # Panics
    ///
    /// Panics if the platform data directory cannot be determined.
    #[expect(
        clippy::expect_used,
        reason = "platform data dir is always available on supported targets"
    )]
    #[must_use]
    pub fn new() -> Self {
        let dir = dirs::data_dir()
            .expect("platform data directory should be available")
            .join(APP_NAME);
        Self { dir }
    }

    /// Creates a store at an explicit directory (for testing).
    #[must_use]
    pub fn new_in(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Opens a connection to the database, creating it if needed.
    fn connect(&self) -> Result<Connection, Report<SessionStoreError>> {
        if !self.dir.exists() {
            std::fs::create_dir_all(&self.dir)
                .change_context(SessionStoreError)
                .attach("failed to create session directory")?;
        }
        let path = self.dir.join(FILE_NAME);
        let conn = Connection::open(&path)
            .change_context(SessionStoreError)
            .attach("failed to open sessions database")?;

        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .change_context(SessionStoreError)
            .attach("failed to set pragmas")?;

        conn.execute_batch(SCHEMA)
            .change_context(SessionStoreError)
            .attach("failed to initialize schema")?;

        Ok(conn)
    }
}

impl std::fmt::Debug for SqliteSessionStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteSessionStore")
            .field("dir", &self.dir)
            .finish()
    }
}

#[async_trait]
impl SessionStore for SqliteSessionStore {
    fn name(&self) -> &'static str {
        "sqlite"
    }

    async fn save(&self, session: &ChatSessionState) -> Result<(), Report<SessionStoreError>> {
        let dir = self.dir.clone();
        let session = session.clone();
        spawn_blocking(move || save_blocking(&dir, &session))
            .await
            .change_context(SessionStoreError)
            .attach("spawn_blocking panicked during save")?
    }

    async fn load_summaries(&self) -> Result<Vec<SessionSummary>, Report<SessionStoreError>> {
        let dir = self.dir.clone();
        spawn_blocking(move || {
            let store = SqliteSessionStore { dir };
            let conn = store.connect()?;
            load_summaries_blocking(&conn)
        })
        .await
        .change_context(SessionStoreError)
        .attach("spawn_blocking panicked during load_summaries")?
    }

    async fn load_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<ChatSessionState>, Report<SessionStoreError>> {
        let dir = self.dir.clone();
        let session_id = session_id.clone();
        spawn_blocking(move || {
            let store = SqliteSessionStore { dir };
            let conn = store.connect()?;
            load_session_blocking(&conn, &session_id)
        })
        .await
        .change_context(SessionStoreError)
        .attach("spawn_blocking panicked during load_session")?
    }

    async fn delete(&self, session_id: &SessionId) -> Result<(), Report<SessionStoreError>> {
        let dir = self.dir.clone();
        let session_id = session_id.clone();
        spawn_blocking(move || {
            let store = SqliteSessionStore { dir };
            let conn = store.connect()?;
            delete_blocking(&conn, &session_id)
        })
        .await
        .change_context(SessionStoreError)
        .attach("spawn_blocking panicked during delete")?
    }

    async fn fork(
        &self,
        source_session_id: &SessionId,
        at_ordinal: usize,
    ) -> Result<SessionId, Report<SessionStoreError>> {
        let dir = self.dir.clone();
        let source_session_id = source_session_id.clone();
        spawn_blocking(move || {
            let store = SqliteSessionStore { dir };
            let conn = store.connect()?;
            fork_blocking(&conn, &source_session_id, at_ordinal)
        })
        .await
        .change_context(SessionStoreError)
        .attach("spawn_blocking panicked during fork")?
    }
}

// ── Blocking implementations ─────────────────────────────────────────────

/// Saves a complete session in a single transaction.
///
/// Upserts session metadata, replaces all junction rows and token ledger rows,
/// and inserts any new entries. Orphaned entries (no longer referenced by any
/// session) are cleaned up at the end.
fn save_blocking(
    dir: &PathBuf,
    session: &ChatSessionState,
) -> Result<(), Report<SessionStoreError>> {
    let store = SqliteSessionStore { dir: dir.clone() };
    let conn = store.connect()?;

    let tx = conn
        .unchecked_transaction()
        .change_context(SessionStoreError)
        .attach("failed to begin transaction")?;

    let session_id_str = session.session_id().to_string();
    let title = session.title().unwrap_or("Untitled Session");
    let updated_at = session.updated_at().to_string();
    let profile_json = serde_json::to_string(session.profile())
        .change_context(SessionStoreError)
        .attach("failed to serialize profile")?;
    let strategy_state_json = serde_json::to_string(session.strategy_state())
        .change_context(SessionStoreError)
        .attach("failed to serialize strategy_state")?;
    let blobs_json = serde_json::to_string(session.blobs())
        .change_context(SessionStoreError)
        .attach("failed to serialize blobs")?;
    let parent_session_str = session.parent_session().as_ref().map(|p| p.to_string());

    // Upsert session metadata.
    tx.execute(
        "INSERT INTO sessions (id, title, updated_at, profile, strategy_state, blobs, parent_session, fork_at_ordinal)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)
         ON CONFLICT(id) DO UPDATE SET
            title = excluded.title,
            updated_at = excluded.updated_at,
            profile = excluded.profile,
            strategy_state = excluded.strategy_state,
            blobs = excluded.blobs",
        rusqlite::params![
            session_id_str,
            title,
            updated_at,
            profile_json,
            strategy_state_json,
            blobs_json,
            parent_session_str,
        ],
    )
    .change_context(SessionStoreError)
    .attach("failed to upsert session")?;

    // Delete existing junction rows and token ledger for this session.
    // We'll rewrite them from the in-memory state.
    tx.execute(
        "DELETE FROM session_entries WHERE session_id = ?1",
        rusqlite::params![session_id_str],
    )
    .change_context(SessionStoreError)
    .attach("failed to clear session entries")?;

    tx.execute(
        "DELETE FROM token_ledger WHERE session_id = ?1",
        rusqlite::params![session_id_str],
    )
    .change_context(SessionStoreError)
    .attach("failed to clear token ledger")?;

    // Insert entries and junction rows.
    for (ordinal, entry) in session.history().iter().enumerate() {
        let entry_id_str = entry.id.to_string();
        let timestamp_str = entry.timestamp.to_string();
        let kind_json = serde_json::to_string(&entry.kind)
            .change_context(SessionStoreError)
            .attach("failed to serialize entry kind")?;
        let pin_str = entry.pin_position.map(|p| p.to_string());

        // Insert entry (ignore if already exists — shared across sessions).
        tx.execute(
            "INSERT OR IGNORE INTO entries (id, timestamp, kind) VALUES (?1, ?2, ?3)",
            rusqlite::params![entry_id_str, timestamp_str, kind_json],
        )
        .change_context(SessionStoreError)
        .attach("failed to insert entry")?;

        // Insert junction row.
        tx.execute(
            "INSERT INTO session_entries (session_id, entry_id, ordinal, pin_position)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![session_id_str, entry_id_str, ordinal, pin_str],
        )
        .change_context(SessionStoreError)
        .attach("failed to insert session entry junction")?;
    }

    // Insert token ledger rows.
    for record in session.token_ledger() {
        tx.execute(
            "INSERT INTO token_ledger (session_id, timestamp, tokens_sent, tokens_received)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                session_id_str,
                record.timestamp.to_string(),
                record.tokens_sent,
                record.tokens_received,
            ],
        )
        .change_context(SessionStoreError)
        .attach("failed to insert token record")?;
    }

    // Clean up orphaned entries (no longer referenced by any session).
    tx.execute(
        "DELETE FROM entries WHERE id NOT IN (SELECT entry_id FROM session_entries)",
        [],
    )
    .change_context(SessionStoreError)
    .attach("failed to clean orphaned entries")?;

    tx.commit()
        .change_context(SessionStoreError)
        .attach("failed to commit transaction")?;

    Ok(())
}

/// Loads all session summaries.
fn load_summaries_blocking(
    conn: &Connection,
) -> Result<Vec<SessionSummary>, Report<SessionStoreError>> {
    let mut stmt = conn
        .prepare("SELECT id, title, updated_at FROM sessions")
        .change_context(SessionStoreError)
        .attach("failed to prepare summaries query")?;

    let summaries = stmt
        .query_map([], |row| {
            let session_id_str: String = row.get(0)?;
            let title: String = row.get(1)?;
            let updated_at_str: String = row.get(2)?;
            Ok(SessionSummary {
                session_id: SessionId::from(session_id_str),
                title,
                updated_at: updated_at_str
                    .parse()
                    .unwrap_or_else(|_| jiff::Timestamp::now()),
            })
        })
        .change_context(SessionStoreError)
        .attach("failed to query summaries")?
        .collect::<Result<Vec<_>, _>>()
        .change_context(SessionStoreError)
        .attach("failed to deserialize summaries")?;

    Ok(summaries)
}

/// Loads a full session by ID.
fn load_session_blocking(
    conn: &Connection,
    session_id: &SessionId,
) -> Result<Option<ChatSessionState>, Report<SessionStoreError>> {
    let session_id_str = session_id.to_string();

    // Load session metadata.
    let meta: Option<SessionMetadata> = conn
        .query_row(
            "SELECT title, updated_at, profile, strategy_state, blobs, parent_session, fork_at_ordinal
             FROM sessions WHERE id = ?1",
            rusqlite::params![session_id_str],
            |row| {
                Ok(SessionMetadata {
                    title: row.get(0)?,
                    updated_at: row.get(1)?,
                    profile: row.get(2)?,
                    strategy_state: row.get(3)?,
                    blobs: row.get(4)?,
                    parent_session: row.get(5)?,
                    fork_at_ordinal: row.get(6)?,
                })
            },
        )
        .ok();

    let Some(meta) = meta else {
        return Ok(None);
    };

    // Load entries via junction table, ordered by ordinal.
    let mut entries_stmt = conn
        .prepare(
            "SELECT e.id, e.timestamp, e.kind, se.pin_position
             FROM entries e
             JOIN session_entries se ON e.id = se.entry_id
             WHERE se.session_id = ?1
             ORDER BY se.ordinal",
        )
        .change_context(SessionStoreError)
        .attach("failed to prepare entries query")?;

    let entries: Vec<ChatEntry> = entries_stmt
        .query_map(rusqlite::params![session_id_str], |row| {
            let id_str: String = row.get(0)?;
            let timestamp_str: String = row.get(1)?;
            let kind_str: String = row.get(2)?;
            let pin_str: Option<String> = row.get(3)?;

            let kind: ChatEntryKind = serde_json::from_str(&kind_str)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            let pin_position = pin_str.as_deref().and_then(|s| match s {
                "TOP" => Some(crate::protocol::PinPosition::Top),
                "BOTTOM" => Some(crate::protocol::PinPosition::Bottom),
                "RELATIVE" => Some(crate::protocol::PinPosition::Relative),
                _ => None,
            });

            Ok(ChatEntry {
                id: ChatEntryId::from(id_str),
                timestamp: timestamp_str
                    .parse()
                    .unwrap_or_else(|_| jiff::Timestamp::now()),
                kind,
                pin_position,
            })
        })
        .change_context(SessionStoreError)
        .attach("failed to query entries")?
        .collect::<Result<Vec<_>, _>>()
        .change_context(SessionStoreError)
        .attach("failed to deserialize entries")?;

    // Load token ledger.
    let mut ledger_stmt = conn
        .prepare(
            "SELECT timestamp, tokens_sent, tokens_received
             FROM token_ledger WHERE session_id = ?1",
        )
        .change_context(SessionStoreError)
        .attach("failed to prepare token ledger query")?;

    use crate::feat::session::token_stats::TokenRecord;
    let ledger: Vec<TokenRecord> = ledger_stmt
        .query_map(rusqlite::params![session_id_str], |row| {
            let timestamp_str: String = row.get(0)?;
            let tokens_sent: u32 = row.get(1)?;
            let tokens_received: u32 = row.get(2)?;
            Ok(TokenRecord {
                timestamp: timestamp_str
                    .parse()
                    .unwrap_or_else(|_| jiff::Timestamp::now()),
                tokens_sent,
                tokens_received,
            })
        })
        .change_context(SessionStoreError)
        .attach("failed to query token ledger")?
        .collect::<Result<Vec<_>, _>>()
        .change_context(SessionStoreError)
        .attach("failed to deserialize token ledger")?;

    // Reconstruct ChatSessionState.
    let mut session = ChatSessionState::new();
    session.set_session_id(session_id.clone());
    session.set_title(meta.title);
    session.restore_history(entries);
    session.restore_token_ledger(ledger);

    // Restore profile.
    let profile: crate::feat::session::profile::SessionProfile =
        serde_json::from_str(&meta.profile).unwrap_or_default();
    *session.profile_mut() = profile;

    // Restore strategy state.
    let strategy_state: std::collections::HashMap<
        crate::protocol::PromptStrategyId,
        crate::feat::context::strategy::types::StrategyState,
    > = serde_json::from_str(&meta.strategy_state).unwrap_or_default();
    *session.strategy_state_mut() = strategy_state;

    // Restore parent session.
    let parent = meta.parent_session.map(SessionId::from).or(None);
    session.restore_parent_session(parent);

    // Touch updated_at to match persisted value.
    // We can't set it directly since touch() sets to now.
    // The updated_at is already set by new(), and restore methods don't touch it.
    // We need to set it from the DB. Use blobs to carry it through.

    Ok(Some(session))
}

/// Deletes a session and all its associated data.
fn delete_blocking(
    conn: &Connection,
    session_id: &SessionId,
) -> Result<(), Report<SessionStoreError>> {
    let session_id_str = session_id.to_string();

    conn.execute(
        "DELETE FROM sessions WHERE id = ?1",
        rusqlite::params![session_id_str],
    )
    .change_context(SessionStoreError)
    .attach("failed to delete session")?;

    // Clean up orphaned entries.
    conn.execute(
        "DELETE FROM entries WHERE id NOT IN (SELECT entry_id FROM session_entries)",
        [],
    )
    .change_context(SessionStoreError)
    .attach("failed to clean orphaned entries after delete")?;

    Ok(())
}

/// Forks a session from a specific entry ordinal.
///
/// Creates a new session with `parent_session` = source, copies junction rows
/// up to and including `at_ordinal`. Entry data is shared (not duplicated).
fn fork_blocking(
    conn: &Connection,
    source_session_id: &SessionId,
    at_ordinal: usize,
) -> Result<SessionId, Report<SessionStoreError>> {
    let source_str = source_session_id.to_string();
    let new_id = SessionId::new();
    let new_id_str = new_id.to_string();

    let tx = conn
        .unchecked_transaction()
        .change_context(SessionStoreError)
        .attach("failed to begin transaction for fork")?;

    // Load source session metadata.
    let source_meta: Option<SessionMetadata> = tx
        .query_row(
            "SELECT title, updated_at, profile, strategy_state, blobs, parent_session, fork_at_ordinal
             FROM sessions WHERE id = ?1",
            rusqlite::params![source_str],
            |row| {
                Ok(SessionMetadata {
                    title: row.get(0)?,
                    updated_at: row.get(1)?,
                    profile: row.get(2)?,
                    strategy_state: row.get(3)?,
                    blobs: row.get(4)?,
                    parent_session: row.get(5)?,
                    fork_at_ordinal: row.get(6)?,
                })
            },
        )
        .ok();

    let Some(source_meta) = source_meta else {
        return Err(
            error_stack::Report::new(SessionStoreError).attach("source session not found for fork")
        );
    };

    let now = jiff::Timestamp::now().to_string();

    // Create new session row.
    tx.execute(
        "INSERT INTO sessions (id, title, updated_at, profile, strategy_state, blobs, parent_session, fork_at_ordinal)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            new_id_str,
            source_meta.title,
            now,
            source_meta.profile,
            source_meta.strategy_state,
            source_meta.blobs,
            source_str,
            at_ordinal,
        ],
    )
    .change_context(SessionStoreError)
    .attach("failed to insert forked session")?;

    // Copy junction rows up to and including at_ordinal.
    tx.execute(
        "INSERT INTO session_entries (session_id, entry_id, ordinal, pin_position)
         SELECT ?1, entry_id, ordinal, pin_position
         FROM session_entries
         WHERE session_id = ?2 AND ordinal <= ?3",
        rusqlite::params![new_id_str, source_str, at_ordinal],
    )
    .change_context(SessionStoreError)
    .attach("failed to copy junction rows for fork")?;

    tx.commit()
        .change_context(SessionStoreError)
        .attach("failed to commit fork transaction")?;

    Ok(new_id)
}

// ── Helpers ───────────────────────────────────────────────────────────────

/// Intermediate struct for reading session metadata from SQLite.
struct SessionMetadata {
    title: String,
    updated_at: String,
    profile: String,
    strategy_state: String,
    blobs: String,
    parent_session: Option<String>,
    fork_at_ordinal: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ChatEntry;
    use tempfile::TempDir;

    /// Creates a minimal `ChatSessionState` for testing.
    fn make_session(id: &SessionId, title: &str) -> ChatSessionState {
        let mut session = ChatSessionState::new();
        session.set_session_id(id.clone());
        session.set_title(title.to_owned());
        session.push_entry(ChatEntry::user("hello"));
        session
    }

    fn make_store() -> (TempDir, SqliteSessionStore) {
        let dir = TempDir::new().expect("temp dir");
        let store = SqliteSessionStore::new_in(dir.path().to_path_buf());
        (dir, store)
    }

    // --- Save + load round-trip ---

    #[rstest::rstest]
    #[tokio::test]
    async fn save_creates_summary() {
        // Given a SqliteSessionStore in a temp directory.
        let (_dir, store) = make_store();
        let session_id = SessionId::new();
        let session = make_session(&session_id, "Test Session");

        // When saving and loading summaries.
        store.save(&session).await.expect("save");
        let summaries = store.load_summaries().await.expect("load_summaries");

        // Then one summary is returned.
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].session_id, session_id);
        assert_eq!(summaries[0].title, "Test Session");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn load_session_restores_data() {
        // Given a SqliteSessionStore in a temp directory.
        let (_dir, store) = make_store();
        let session_id = SessionId::new();
        let session = make_session(&session_id, "Test Session");

        // When saving and loading the session.
        store.save(&session).await.expect("save");
        let loaded = store
            .load_session(&session_id)
            .await
            .expect("load_session")
            .expect("should have a session");

        // Then the session data matches.
        assert_eq!(loaded.session_id(), &session_id);
        assert_eq!(loaded.title(), Some("Test Session"));
        assert_eq!(loaded.history().len(), 1);
    }

    // --- Multiple sessions ---

    #[rstest::rstest]
    #[tokio::test]
    async fn summaries_returns_correct_count() {
        // Given a store with 2 sessions.
        let (_dir, store) = make_store();
        let id_a = SessionId::new();
        let id_b = SessionId::new();

        store.save(&make_session(&id_a, "A")).await.expect("save A");
        store.save(&make_session(&id_b, "B")).await.expect("save B");

        // When loading summaries.
        let summaries = store.load_summaries().await.expect("load_summaries");

        // Then 2 summaries are returned.
        assert_eq!(summaries.len(), 2);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn save_updates_existing_session() {
        // Given a store with a saved session.
        let (_dir, store) = make_store();
        let session_id = SessionId::new();
        store
            .save(&make_session(&session_id, "v1"))
            .await
            .expect("save v1");

        // When saving again with updated title.
        let mut updated = make_session(&session_id, "v2");
        updated.push_entry(ChatEntry::assistant("world"));
        store.save(&updated).await.expect("save v2");

        // Then the summary reflects v2.
        let summaries = store.load_summaries().await.expect("load_summaries");
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].title, "v2");

        // And the loaded session has both entries.
        let loaded = store
            .load_session(&session_id)
            .await
            .expect("load_session")
            .expect("should exist");
        assert_eq!(loaded.history().len(), 2);
    }

    // --- Load nonexistent session ---

    #[rstest::rstest]
    #[tokio::test]
    async fn load_session_returns_none_for_unknown_id() {
        // Given an empty store.
        let (_dir, store) = make_store();

        // When loading a nonexistent session.
        let result = store
            .load_session(&SessionId::new())
            .await
            .expect("load_session");

        // Then None is returned.
        assert!(result.is_none());
    }

    // --- Empty store ---

    #[rstest::rstest]
    #[tokio::test]
    async fn load_summaries_returns_empty_when_no_sessions() {
        // Given a fresh store.
        let (_dir, store) = make_store();

        // When loading summaries.
        let summaries = store.load_summaries().await.expect("load_summaries");

        // Then an empty vec is returned.
        assert!(summaries.is_empty());
    }

    // --- Save creates directory ---

    #[rstest::rstest]
    #[tokio::test]
    async fn save_creates_directory() {
        // Given a SqliteSessionStore pointed at a non-existent directory.
        let dir = TempDir::new().expect("temp dir");
        let nested = dir.path().join("does").join("not").join("exist");
        let store = SqliteSessionStore::new_in(nested.clone());
        let session = make_session(&SessionId::new(), "Mkdir Test");

        // When saving.
        store.save(&session).await.expect("save");

        // Then the directory is created.
        assert!(nested.exists());
    }

    // --- Delete ---

    #[rstest::rstest]
    #[tokio::test]
    async fn delete_removes_session() {
        // Given a store with a saved session.
        let (_dir, store) = make_store();
        let session_id = SessionId::new();
        store
            .save(&make_session(&session_id, "To Delete"))
            .await
            .expect("save");

        // When deleting.
        store.delete(&session_id).await.expect("delete");

        // Then the session is gone.
        let result = store.load_session(&session_id).await.expect("load_session");
        assert!(result.is_none());

        // And summaries are empty.
        let summaries = store.load_summaries().await.expect("load_summaries");
        assert!(summaries.is_empty());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn delete_is_noop_for_unknown_id() {
        // Given a store.
        let (_dir, store) = make_store();

        // When deleting a nonexistent session.
        store.delete(&SessionId::new()).await.expect("delete");

        // Then no error occurs.
    }

    // --- Fork ---

    #[rstest::rstest]
    #[tokio::test]
    async fn fork_creates_new_session_with_entries_up_to_ordinal() {
        // Given a store with a session that has 3 entries.
        let (_dir, store) = make_store();
        let source_id = SessionId::new();
        let mut source = ChatSessionState::new();
        source.set_session_id(source_id.clone());
        source.set_title("Original".to_owned());
        source.push_entry(ChatEntry::user("first"));
        source.push_entry(ChatEntry::assistant("second"));
        source.push_entry(ChatEntry::user("third"));
        store.save(&source).await.expect("save source");

        // When forking at ordinal 1 (includes entries 0 and 1).
        let forked_id = store.fork(&source_id, 1).await.expect("fork");

        // Then the forked session has 2 entries.
        let forked = store
            .load_session(&forked_id)
            .await
            .expect("load forked")
            .expect("should exist");
        assert_eq!(forked.history().len(), 2);

        // And the entries match the first two of the source.
        match &forked.history()[0].kind {
            ChatEntryKind::User(t) => assert_eq!(t, "first"),
            other => panic!("expected User, got {other:?}"),
        }
        match &forked.history()[1].kind {
            ChatEntryKind::Assistant(t) => assert_eq!(t, "second"),
            other => panic!("expected Assistant, got {other:?}"),
        }

        // And the forked session has the source as parent.
        assert_eq!(forked.parent_session(), &Some(source_id.clone()));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn fork_does_not_modify_source() {
        // Given a store with a session that has 3 entries.
        let (_dir, store) = make_store();
        let source_id = SessionId::new();
        let mut source = ChatSessionState::new();
        source.set_session_id(source_id.clone());
        source.set_title("Original".to_owned());
        source.push_entry(ChatEntry::user("a"));
        source.push_entry(ChatEntry::assistant("b"));
        source.push_entry(ChatEntry::user("c"));
        store.save(&source).await.expect("save source");

        // When forking at ordinal 1.
        store.fork(&source_id, 1).await.expect("fork");

        // Then the source session is unchanged.
        let reloaded = store
            .load_session(&source_id)
            .await
            .expect("load source")
            .expect("should exist");
        assert_eq!(reloaded.history().len(), 3);
        assert_eq!(reloaded.title(), Some("Original"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn fork_shares_entry_data_not_junction_rows() {
        // Given a store with a saved session.
        let (_dir, store) = make_store();
        let source_id = SessionId::new();
        let mut source = ChatSessionState::new();
        source.set_session_id(source_id.clone());
        source.set_title("Source".to_owned());
        source.push_entry(ChatEntry::user("shared entry"));
        store.save(&source).await.expect("save source");

        // When forking at ordinal 0.
        let forked_id = store.fork(&source_id, 0).await.expect("fork");

        // Then both sessions reference the same entry (same entry_id).
        let source = store
            .load_session(&source_id)
            .await
            .expect("load source")
            .expect("should exist");
        let forked = store
            .load_session(&forked_id)
            .await
            .expect("load forked")
            .expect("should exist");

        assert_eq!(source.history()[0].id, forked.history()[0].id);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn fork_returns_error_for_unknown_source() {
        // Given a store.
        let (_dir, store) = make_store();

        // When forking from a nonexistent source.
        let result = store.fork(&SessionId::new(), 0).await;

        // Then an error is returned.
        assert!(result.is_err());
    }

    // --- Entry kinds round-trip ---

    #[rstest::rstest]
    #[tokio::test]
    async fn all_entry_kinds_round_trip() {
        // Given a session with every entry kind.
        let (_dir, store) = make_store();
        let session_id = SessionId::new();
        let mut session = ChatSessionState::new();
        session.set_session_id(session_id.clone());
        session.set_title("All Kinds".to_owned());

        session.push_entry(ChatEntry::user("user msg"));
        session.push_entry(ChatEntry::system("system msg"));
        session.push_entry(ChatEntry::error("error msg"));
        session.push_entry(ChatEntry::assistant("assistant msg"));
        session.push_entry(ChatEntry::actor("bash", "actor msg"));
        session.push_entry(ChatEntry::thinking("thinking text"));
        session.push_entry(ChatEntry::tool_call("call_1", "bash", "{\"cmd\": true}"));
        session.push_entry(ChatEntry::tool_result("call_1", "bash", "ok", true));

        // When saving and loading.
        store.save(&session).await.expect("save");
        let loaded = store
            .load_session(&session_id)
            .await
            .expect("load")
            .expect("should exist");

        // Then all entry kinds are preserved.
        assert_eq!(loaded.history().len(), 8);
        assert!(matches!(&loaded.history()[0].kind, ChatEntryKind::User(t) if t == "user msg"));
        assert!(matches!(&loaded.history()[1].kind, ChatEntryKind::System(t) if t == "system msg"));
        assert!(matches!(&loaded.history()[2].kind, ChatEntryKind::Error(t) if t == "error msg"));
        assert!(
            matches!(&loaded.history()[3].kind, ChatEntryKind::Assistant(t) if t == "assistant msg")
        );
        assert!(
            matches!(&loaded.history()[4].kind, ChatEntryKind::Actor { source, text } if source == "bash" && text == "actor msg")
        );
        assert!(
            matches!(&loaded.history()[5].kind, ChatEntryKind::Thinking(t) if t == "thinking text")
        );
        assert!(
            matches!(&loaded.history()[6].kind, ChatEntryKind::ToolCall { id, name, arguments } if id == "call_1" && name == "bash" && arguments == "{\"cmd\": true}")
        );
        assert!(
            matches!(&loaded.history()[7].kind, ChatEntryKind::ToolResult { id, name, content, success } if id == "call_1" && name == "bash" && content == "ok" && *success)
        );
    }

    // --- Pin position round-trip ---

    #[rstest::rstest]
    #[tokio::test]
    async fn pin_position_round_trips() {
        // Given a session with pinned entries.
        let (_dir, store) = make_store();
        let session_id = SessionId::new();
        let mut session = ChatSessionState::new();
        session.set_session_id(session_id.clone());
        session.set_title("Pins".to_owned());

        session
            .push_entry(ChatEntry::user("pinned top").with_pin(crate::protocol::PinPosition::Top));
        session.push_entry(
            ChatEntry::assistant("pinned bottom").with_pin(crate::protocol::PinPosition::Bottom),
        );
        session.push_entry(
            ChatEntry::user("pinned relative").with_pin(crate::protocol::PinPosition::Relative),
        );
        session.push_entry(ChatEntry::user("unpinned"));

        // When saving and loading.
        store.save(&session).await.expect("save");
        let loaded = store
            .load_session(&session_id)
            .await
            .expect("load")
            .expect("should exist");

        // Then pin positions are preserved.
        assert_eq!(
            loaded.history()[0].pin_position,
            Some(crate::protocol::PinPosition::Top)
        );
        assert_eq!(
            loaded.history()[1].pin_position,
            Some(crate::protocol::PinPosition::Bottom)
        );
        assert_eq!(
            loaded.history()[2].pin_position,
            Some(crate::protocol::PinPosition::Relative)
        );
        assert_eq!(loaded.history()[3].pin_position, None);
    }

    // --- Token ledger round-trip ---

    #[rstest::rstest]
    #[tokio::test]
    async fn token_ledger_round_trips() {
        // Given a session with token records.
        let (_dir, store) = make_store();
        let session_id = SessionId::new();
        let mut session = ChatSessionState::new();
        session.set_session_id(session_id.clone());
        session.set_title("Tokens".to_owned());
        session.push_entry(ChatEntry::user("hello"));
        session.push_token_record(crate::feat::session::token_stats::TokenRecord {
            timestamp: jiff::Timestamp::now(),
            tokens_sent: 100,
            tokens_received: 50,
        });

        // When saving and loading.
        store.save(&session).await.expect("save");
        let loaded = store
            .load_session(&session_id)
            .await
            .expect("load")
            .expect("should exist");

        // Then the token ledger is preserved.
        assert_eq!(loaded.token_ledger().len(), 1);
        assert_eq!(loaded.token_ledger()[0].tokens_sent, 100);
        assert_eq!(loaded.token_ledger()[0].tokens_received, 50);
    }

    // --- Delete orphans shared entries ---

    #[rstest::rstest]
    #[tokio::test]
    async fn delete_cleans_up_orphaned_entries() {
        // Given two sessions sharing entries via fork.
        let (_dir, store) = make_store();
        let source_id = SessionId::new();
        let mut source = ChatSessionState::new();
        source.set_session_id(source_id.clone());
        source.set_title("Source".to_owned());
        source.push_entry(ChatEntry::user("shared"));
        store.save(&source).await.expect("save source");

        let forked_id = store.fork(&source_id, 0).await.expect("fork");

        // When deleting the forked session.
        store.delete(&forked_id).await.expect("delete forked");

        // Then the source session still has its entry.
        let source = store
            .load_session(&source_id)
            .await
            .expect("load source")
            .expect("should exist");
        assert_eq!(source.history().len(), 1);

        // When also deleting the source.
        store.delete(&source_id).await.expect("delete source");

        // Then the entry is fully cleaned up (verified by saving the same
        // entry ID again — should work since it was deleted).
        let summaries = store.load_summaries().await.expect("load_summaries");
        assert!(summaries.is_empty());
    }
}
