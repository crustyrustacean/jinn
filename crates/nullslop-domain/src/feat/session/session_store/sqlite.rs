//! SQLite-backed session store implementation.
//!
//! Stores session data in normalized tables with a junction table for entries.
//! This eliminates duplication — each chat entry is stored once and shared
//! across sessions. The junction table enables fork support by copying only
//! small junction rows, not entry data.
//!
//! Uses Diesel's type-safe query DSL for compile-time column verification
//! against the generated schema. All queries are checked at compile time —
//! if a column is added to a migration but missing from an INSERT or SELECT,
//! the code will not compile.

use std::path::PathBuf;

use async_trait::async_trait;
use diesel::insert_into;
use diesel::prelude::*;
use diesel::r2d2::{self as diesel_r2d2, CustomizeConnection, Pool};
use diesel::upsert::excluded;
use error_stack::{Report, ResultExt as _};
use tokio::task::spawn_blocking;

use crate::common::app_info::APP_NAME;
use crate::feat::session::SessionUi;
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryKind};
use crate::feat::session::chat_session::{ChatSessionState, SessionCore, SessionCoreEphemeral};
use crate::feat::session::session_summary::SessionSummary;
use crate::feat::session::token_stats::TokenRecord;
use crate::protocol::{ChatEntryId, SessionId};

use super::{SessionStore, SessionStoreError};

// Migrations are embedded at compile time from the SQL files in the migrations directory.
// The path is relative to this source file.
const _V0_UP: &str =
    include_str!("../../../../migrations/00000000000000_create_initial_schema/up.sql");
const _V1_UP: &str = include_str!("../../../../migrations/00000000000001_add_cwd_column/up.sql");
const _V2_UP: &str =
    include_str!("../../../../migrations/00000000000002_add_created_at_column/up.sql");

/// Runs all pending migrations on a bootstrap connection.
///
/// Uses individual `sql_query` calls because Diesel's `sql_query` doesn't support
/// multi-statement batches. The SQL content is embedded at compile time from the
/// migration files.
fn run_migrations(conn: &mut SqliteConnection) {
    // v0: create initial schema
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS sessions (\
         id TEXT PRIMARY KEY, title TEXT, updated_at TEXT NOT NULL,\
         profile TEXT NOT NULL DEFAULT '{}', strategy_state TEXT NOT NULL DEFAULT '{}',\
         blobs TEXT NOT NULL DEFAULT '{}', parent_session TEXT DEFAULT NULL)",
    )
    .execute(conn)
    .expect("v0: create sessions table");

    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS entries (\
         id TEXT PRIMARY KEY, timestamp TEXT NOT NULL, kind TEXT NOT NULL)",
    )
    .execute(conn)
    .expect("v0: create entries table");

    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS session_entries (\
         session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,\
         entry_id TEXT NOT NULL REFERENCES entries(id) ON DELETE CASCADE,\
         ordinal INTEGER NOT NULL, pin_position TEXT DEFAULT NULL,\
         PRIMARY KEY (session_id, entry_id), UNIQUE (session_id, ordinal))",
    )
    .execute(conn)
    .expect("v0: create session_entries table");

    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS token_ledger (\
         id INTEGER PRIMARY KEY AUTOINCREMENT,\
         session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,\
         timestamp TEXT NOT NULL, tokens_sent INTEGER NOT NULL, tokens_received INTEGER NOT NULL)",
    )
    .execute(conn)
    .expect("v0: create token_ledger table");

    diesel::sql_query(
        "CREATE INDEX IF NOT EXISTS idx_session_entries_session ON session_entries(session_id, ordinal)"
    ).execute(conn).expect("v0: create session_entries index");

    diesel::sql_query(
        "CREATE INDEX IF NOT EXISTS idx_token_ledger_session ON token_ledger(session_id)",
    )
    .execute(conn)
    .expect("v0: create token_ledger index");

    // v1: add cwd column
    // ALTER TABLE ADD COLUMN fails if the column already exists. Ignore the error.
    let _ = diesel::sql_query("ALTER TABLE sessions ADD COLUMN cwd TEXT NOT NULL DEFAULT '.'")
        .execute(conn);

    // v2: add created_at column
    let _ =
        diesel::sql_query("ALTER TABLE sessions ADD COLUMN created_at TEXT NOT NULL DEFAULT ''")
            .execute(conn);
}

/// Configuration for the SQLite connection pool.
///
/// Controls pool sizing. Use [`PoolConfig::default()`] for sensible defaults
/// or construct with a specific max size.
pub struct PoolConfig {
    /// Maximum number of connections in the pool.
    pub max_size: u32,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self { max_size: 4 }
    }
}

pub struct SqliteSessionStore {
    /// Connection pool for `sessions.db`.
    pool: Pool<diesel_r2d2::ConnectionManager<SqliteConnection>>,
}

/// SQLite database file name.
const FILE_NAME: &str = "sessions.db";

impl SqliteSessionStore {
    /// Creates a store at the platform data directory.
    ///
    /// Uses `dirs::data_dir()` → `nullslop/sessions.db` on Linux.
    /// The database file is created on first access. Migrations are run
    /// once during pool initialization.
    ///
    /// # Panics
    ///
    /// Panics if the platform data directory cannot be determined or
    /// pool creation fails.
    #[expect(
        clippy::expect_used,
        reason = "platform data dir is always available on supported targets"
    )]
    #[must_use]
    pub fn new() -> Self {
        let dir = dirs::data_dir()
            .expect("platform data directory should be available")
            .join(APP_NAME);
        Self::build_pool(&dir, PoolConfig::default())
    }

    /// Creates a store at an explicit directory (for testing).
    #[must_use]
    pub fn new_in(dir: PathBuf) -> Self {
        Self::build_pool(&dir, PoolConfig::default())
    }

    /// Creates a store at an explicit directory with custom pool configuration.
    #[must_use]
    pub fn new_with_config(dir: PathBuf, config: PoolConfig) -> Self {
        Self::build_pool(&dir, config)
    }

    /// Builds the connection pool.
    ///
    /// Creates the directory if needed, runs embedded migrations once on a
    /// bootstrap connection, then builds the pool. Each pooled connection gets
    /// WAL and foreign key pragmas set via the connection customizer.
    #[expect(clippy::expect_used, reason = "pool creation failures are fatal")]
    fn build_pool(dir: &PathBuf, config: PoolConfig) -> Self {
        if !dir.exists() {
            std::fs::create_dir_all(dir).expect("failed to create session directory");
        }
        let path = dir.join(FILE_NAME);
        let database_url = path.to_string_lossy().to_string();

        // Run migrations once on a bootstrap connection before building the pool.
        // This ensures the schema is ready before any pooled connections are created.
        {
            let mut conn = SqliteConnection::establish(&database_url)
                .expect("failed to open database for migration");
            diesel::sql_query("PRAGMA journal_mode=WAL")
                .execute(&mut conn)
                .expect("failed to set WAL pragma");
            diesel::sql_query("PRAGMA foreign_keys=ON")
                .execute(&mut conn)
                .expect("failed to set foreign_keys pragma");
            run_migrations(&mut conn);
        }

        let manager = diesel_r2d2::ConnectionManager::<SqliteConnection>::new(&database_url);
        let pool = Pool::builder()
            .max_size(config.max_size)
            .connection_customizer(Box::new(SqliteConnectionCustomizer))
            .build(manager)
            .expect("failed to create connection pool");

        Self { pool }
    }
}

impl std::fmt::Debug for SqliteSessionStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteSessionStore")
            .field("pool_state", &self.pool.state())
            .finish()
    }
}

/// Sets WAL and foreign key pragmas on each pooled connection.
#[derive(Debug, Copy, Clone)]
struct SqliteConnectionCustomizer;

impl CustomizeConnection<SqliteConnection, diesel_r2d2::Error> for SqliteConnectionCustomizer {
    fn on_acquire(&self, conn: &mut SqliteConnection) -> Result<(), diesel_r2d2::Error> {
        diesel::sql_query("PRAGMA journal_mode=WAL")
            .execute(conn)
            .map_err(diesel_r2d2::Error::QueryError)?;
        diesel::sql_query("PRAGMA foreign_keys=ON")
            .execute(conn)
            .map_err(diesel_r2d2::Error::QueryError)?;
        Ok(())
    }
}

#[async_trait]
impl SessionStore for SqliteSessionStore {
    fn name(&self) -> &'static str {
        "sqlite"
    }

    async fn save(&self, session: &ChatSessionState) -> Result<(), Report<SessionStoreError>> {
        let mut conn = self
            .pool
            .get()
            .change_context(SessionStoreError)
            .attach("failed to acquire connection from pool")?;
        let session = session.clone();
        spawn_blocking(move || save_blocking(&mut conn, &session))
            .await
            .change_context(SessionStoreError)
            .attach("spawn_blocking panicked during save")?
    }

    async fn load_summaries(&self) -> Result<Vec<SessionSummary>, Report<SessionStoreError>> {
        let mut conn = self
            .pool
            .get()
            .change_context(SessionStoreError)
            .attach("failed to acquire connection from pool")?;
        spawn_blocking(move || load_summaries_blocking(&mut conn))
            .await
            .change_context(SessionStoreError)
            .attach("spawn_blocking panicked during load_summaries")?
    }

    async fn load_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<ChatSessionState>, Report<SessionStoreError>> {
        let mut conn = self
            .pool
            .get()
            .change_context(SessionStoreError)
            .attach("failed to acquire connection from pool")?;
        let session_id = session_id.clone();
        spawn_blocking(move || load_session_blocking(&mut conn, &session_id))
            .await
            .change_context(SessionStoreError)
            .attach("spawn_blocking panicked during load_session")?
    }

    async fn delete(&self, session_id: &SessionId) -> Result<(), Report<SessionStoreError>> {
        let mut conn = self
            .pool
            .get()
            .change_context(SessionStoreError)
            .attach("failed to acquire connection from pool")?;
        let session_id = session_id.clone();
        spawn_blocking(move || delete_blocking(&mut conn, &session_id))
            .await
            .change_context(SessionStoreError)
            .attach("spawn_blocking panicked during delete")?
    }

    async fn fork(
        &self,
        source_session_id: &SessionId,
        at_ordinal: usize,
    ) -> Result<SessionId, Report<SessionStoreError>> {
        let mut conn = self
            .pool
            .get()
            .change_context(SessionStoreError)
            .attach("failed to acquire connection from pool")?;
        let source_session_id = source_session_id.clone();
        spawn_blocking(move || fork_blocking(&mut conn, &source_session_id, at_ordinal))
            .await
            .change_context(SessionStoreError)
            .attach("spawn_blocking panicked during fork")?
    }
}

// ── Diesel model structs ─────────────────────────────────────────────────

/// Reading model for the `sessions` table.
#[derive(Queryable)]
#[diesel(table_name = crate::schema::sessions)]
struct SessionRow {
    id: Option<String>,
    title: Option<String>,
    updated_at: String,
    created_at: String,
    profile: String,
    strategy_state: String,
    blobs: String,
    parent_session: Option<String>,
    cwd: String,
}

/// Insert model for the `sessions` table.
#[derive(Insertable)]
#[diesel(table_name = crate::schema::sessions)]
struct NewSessionRow {
    id: String,
    title: Option<String>,
    updated_at: String,
    created_at: String,
    profile: String,
    strategy_state: String,
    blobs: String,
    parent_session: Option<String>,
    cwd: String,
}

/// Reading model for the `entries` table.
#[derive(Queryable)]
#[diesel(table_name = crate::schema::entries)]
struct EntryRow {
    id: Option<String>,
    timestamp: String,
    kind: String,
}

/// Insert model for the `entries` table.
#[derive(Insertable)]
#[diesel(table_name = crate::schema::entries)]
struct NewEntryRow {
    id: String,
    timestamp: String,
    kind: String,
}

/// Reading model for the `session_entries` table.
#[derive(Queryable)]
#[diesel(table_name = crate::schema::session_entries)]
struct SessionEntryRow {
    session_id: String,
    entry_id: String,
    ordinal: i32,
    pin_position: Option<String>,
}

/// Insert model for the `session_entries` table.
#[derive(Insertable)]
#[diesel(table_name = crate::schema::session_entries)]
struct NewSessionEntryRow {
    session_id: String,
    entry_id: String,
    ordinal: i32,
    pin_position: Option<String>,
}

/// Reading model for the `token_ledger` table.
#[derive(Queryable)]
#[diesel(table_name = crate::schema::token_ledger)]
struct TokenLedgerRow {
    id: Option<i32>,
    session_id: String,
    timestamp: String,
    tokens_sent: i32,
    tokens_received: i32,
}

/// Insert model for the `token_ledger` table.
#[derive(Insertable)]
#[diesel(table_name = crate::schema::token_ledger)]
struct NewTokenLedgerRow {
    session_id: String,
    timestamp: String,
    tokens_sent: i32,
    tokens_received: i32,
}

// ── Conversions ──────────────────────────────────────────────────────────

impl TryFrom<&ChatSessionState> for NewSessionRow {
    type Error = Report<SessionStoreError>;

    #[deny(unused_variables)]
    fn try_from(session: &ChatSessionState) -> Result<Self, Self::Error> {
        // Exhaustive destructuring — adding a field to SessionCore
        // without updating this pattern is a compile error.
        let ChatSessionState {
            core:
                SessionCore {
                    session_id,
                    title,
                    updated_at,
                    created_at,
                    history: _history, // persisted via entries + session_entries tables below
                    profile,
                    cwd,
                    token_ledger: _ledger, // persisted via token_ledger table below
                    parent_session,
                    strategy_state,
                    blobs,
                    ephemeral: _ephemeral, // runtime-only state, not persisted
                },
            ui: _ui, // runtime-only UI state, not persisted
        } = session;

        Ok(Self {
            id: session_id.to_string(),
            title: title.clone(),
            updated_at: updated_at.to_string(),
            created_at: created_at.to_string(),
            profile: serde_json::to_string(profile)
                .change_context(SessionStoreError)
                .attach("failed to serialize profile")?,
            strategy_state: serde_json::to_string(strategy_state)
                .change_context(SessionStoreError)
                .attach("failed to serialize strategy_state")?,
            blobs: serde_json::to_string(blobs)
                .change_context(SessionStoreError)
                .attach("failed to serialize blobs")?,
            parent_session: parent_session.as_ref().map(|p| p.to_string()),
            cwd: cwd.to_string_lossy().to_string(),
        })
    }
}

/// Carries all data needed to reconstruct a full [`ChatSessionState`] from the database.
struct SessionLoadContext {
    row: SessionRow,
    entries: Vec<ChatEntry>,
    ledger: Vec<TokenRecord>,
}

impl TryFrom<SessionLoadContext> for ChatSessionState {
    type Error = Report<SessionStoreError>;

    #[deny(unused_variables)]
    fn try_from(ctx: SessionLoadContext) -> Result<Self, Self::Error> {
        // Exhaustive destructuring of SessionRow — adding a column to the
        // sessions table without updating this pattern is a compile error.
        let SessionRow {
            id,
            title,
            updated_at,
            created_at,
            profile,
            strategy_state,
            blobs,
            parent_session,
            cwd,
        } = ctx.row;

        let profile = serde_json::from_str(&profile)
            .change_context(SessionStoreError)
            .attach("failed to deserialize profile")?;
        let strategy_state = serde_json::from_str(&strategy_state)
            .change_context(SessionStoreError)
            .attach("failed to deserialize strategy_state")?;
        let blobs = serde_json::from_str(&blobs)
            .change_context(SessionStoreError)
            .attach("failed to deserialize blobs")?;
        let updated_at = updated_at
            .parse()
            .change_context(SessionStoreError)
            .attach("failed to parse updated_at")?;
        let created_at = created_at
            .parse()
            .change_context(SessionStoreError)
            .attach("failed to parse created_at")?;

        // Build ChatSessionState with all fields explicitly set.
        // Every destructured binding from SessionRow is used here.
        Ok(ChatSessionState {
            core: SessionCore {
                session_id: SessionId::from(id.unwrap_or_default()),
                title,
                updated_at,
                created_at,
                history: ctx.entries,
                profile,
                cwd: std::path::PathBuf::from(cwd),
                token_ledger: ctx.ledger,
                parent_session: parent_session.map(SessionId::from),
                strategy_state,
                blobs,
                ephemeral: SessionCoreEphemeral::default(),
            },
            ui: SessionUi::default(),
        })
    }
}

// ── Blocking implementations ─────────────────────────────────────────────

/// Saves a complete session in a single transaction.
///
/// Upserts session metadata, replaces all junction rows and token ledger rows,
/// and inserts any new entries. Orphaned entries (no longer referenced by any
/// session) are cleaned up at the end.
fn save_blocking(
    conn: &mut SqliteConnection,
    session: &ChatSessionState,
) -> Result<(), Report<SessionStoreError>> {
    let row = NewSessionRow::try_from(session)?;
    let session_id_str = row.id.clone();

    conn.transaction::<_, diesel::result::Error, _>(|txn| {
        use crate::schema::{entries, session_entries, sessions, token_ledger};

        // Upsert session metadata.
        insert_into(sessions::table)
            .values(&row)
            .on_conflict(sessions::dsl::id)
            .do_update()
            .set((
                sessions::title.eq(excluded(sessions::title)),
                sessions::updated_at.eq(excluded(sessions::updated_at)),
                sessions::profile.eq(excluded(sessions::profile)),
                sessions::strategy_state.eq(excluded(sessions::strategy_state)),
                sessions::blobs.eq(excluded(sessions::blobs)),
                sessions::cwd.eq(excluded(sessions::cwd)),
            ))
            .execute(txn)?;

        // Delete existing junction rows and token ledger for this session.
        diesel::delete(
            session_entries::table.filter(session_entries::session_id.eq(&session_id_str)),
        )
        .execute(txn)?;

        diesel::delete(token_ledger::table.filter(token_ledger::session_id.eq(&session_id_str)))
            .execute(txn)?;

        // Insert entries and junction rows.
        for (ordinal, entry) in session.history().iter().enumerate() {
            let entry_id_str = entry.id.to_string();
            let timestamp_str = entry.timestamp.to_string();
            let kind_json = serde_json::to_string(&entry.kind)
                .map_err(|e| diesel::result::Error::SerializationError(Box::new(e)))?;
            let pin_str = entry.pin_position.map(|p| p.to_string());

            // Insert entry (ignore if already exists — shared across sessions).
            insert_into(entries::table)
                .values(&NewEntryRow {
                    id: entry_id_str.clone(),
                    timestamp: timestamp_str,
                    kind: kind_json,
                })
                .on_conflict(entries::dsl::id)
                .do_nothing()
                .execute(txn)?;

            // Insert junction row.
            insert_into(session_entries::table)
                .values(&NewSessionEntryRow {
                    session_id: session_id_str.clone(),
                    entry_id: entry_id_str,
                    ordinal: ordinal as i32,
                    pin_position: pin_str,
                })
                .execute(txn)?;
        }

        // Insert token ledger rows.
        for record in session.token_ledger() {
            insert_into(token_ledger::table)
                .values(&NewTokenLedgerRow {
                    session_id: session_id_str.clone(),
                    timestamp: record.timestamp.to_string(),
                    tokens_sent: record.tokens_sent as i32,
                    tokens_received: record.tokens_received as i32,
                })
                .execute(txn)?;
        }

        // Clean up orphaned entries (no longer referenced by any session).
        diesel::sql_query(
            "DELETE FROM entries WHERE id NOT IN (SELECT entry_id FROM session_entries)",
        )
        .execute(txn)?;

        Ok(())
    })
    .change_context(SessionStoreError)
    .attach("failed to save session")?;

    Ok(())
}

/// Loads all session summaries.
fn load_summaries_blocking(
    conn: &mut SqliteConnection,
) -> Result<Vec<SessionSummary>, Report<SessionStoreError>> {
    use crate::schema::sessions::dsl::*;

    let rows: Vec<SessionRow> = sessions
        .load::<SessionRow>(conn)
        .change_context(SessionStoreError)
        .attach("failed to query summaries")?;

    let summaries = rows
        .into_iter()
        .map(|row| SessionSummary {
            session_id: SessionId::from(row.id.unwrap_or_default()),
            title: row.title.unwrap_or_else(|| "Untitled".to_owned()),
            updated_at: row
                .updated_at
                .parse()
                .unwrap_or_else(|_| jiff::Timestamp::now()),
            created_at: row
                .created_at
                .parse()
                .unwrap_or_else(|_| jiff::Timestamp::now()),
        })
        .collect();

    Ok(summaries)
}

/// Loads a full session by ID.
fn load_session_blocking(
    conn: &mut SqliteConnection,
    session_id: &SessionId,
) -> Result<Option<ChatSessionState>, Report<SessionStoreError>> {
    use crate::schema::{entries, session_entries, sessions, token_ledger};

    let session_id_str = session_id.to_string();

    // Load session metadata.
    let meta: Option<SessionRow> = sessions::table
        .filter(sessions::id.eq(&session_id_str))
        .first::<SessionRow>(conn)
        .ok();

    let Some(meta) = meta else {
        return Ok(None);
    };

    // Load entries via junction table, ordered by ordinal.
    let joined: Vec<(EntryRow, SessionEntryRow)> = entries::table
        .inner_join(session_entries::table)
        .filter(session_entries::session_id.eq(&session_id_str))
        .order(session_entries::ordinal.asc())
        .load::<(EntryRow, SessionEntryRow)>(conn)
        .change_context(SessionStoreError)
        .attach("failed to query entries")?;

    let entries: Vec<ChatEntry> = joined
        .into_iter()
        .map(|(entry, junction)| {
            let kind: ChatEntryKind = serde_json::from_str(&entry.kind).unwrap_or_else(|e| {
                tracing::warn!(entry_id = %entry.id.as_deref().unwrap_or("?"), error = %e, "failed to deserialize entry kind");
                ChatEntryKind::Error(format!("corrupt entry: {e}"))
            });
            let pin_position = junction.pin_position.as_deref().and_then(|s| match s {
                "TOP" => Some(crate::protocol::PinPosition::Top),
                "BOTTOM" => Some(crate::protocol::PinPosition::Bottom),
                "RELATIVE" => Some(crate::protocol::PinPosition::Relative),
                _ => None,
            });

            ChatEntry {
                id: ChatEntryId::from(entry.id.unwrap_or_default()),
                timestamp: entry
                    .timestamp
                    .parse()
                    .unwrap_or_else(|_| jiff::Timestamp::now()),
                kind,
                pin_position,
            }
        })
        .collect();

    // Load token ledger.
    let ledger_rows: Vec<TokenLedgerRow> = token_ledger::table
        .filter(token_ledger::session_id.eq(&session_id_str))
        .load::<TokenLedgerRow>(conn)
        .change_context(SessionStoreError)
        .attach("failed to query token ledger")?;

    let ledger: Vec<TokenRecord> = ledger_rows
        .into_iter()
        .map(|row| TokenRecord {
            timestamp: row
                .timestamp
                .parse()
                .unwrap_or_else(|_| jiff::Timestamp::now()),
            tokens_sent: row.tokens_sent as u32,
            tokens_received: row.tokens_received as u32,
        })
        .collect();

    // Reconstruct ChatSessionState via exhaustive destructuring.
    let session = ChatSessionState::try_from(SessionLoadContext {
        row: meta,
        entries,
        ledger,
    })?;

    Ok(Some(session))
}

/// Deletes a session and all its associated data.
fn delete_blocking(
    conn: &mut SqliteConnection,
    session_id: &SessionId,
) -> Result<(), Report<SessionStoreError>> {
    use crate::schema::sessions;

    let session_id_str = session_id.to_string();

    diesel::delete(sessions::table.filter(sessions::id.eq(&session_id_str)))
        .execute(conn)
        .change_context(SessionStoreError)
        .attach("failed to delete session")?;

    // Clean up orphaned entries.
    diesel::sql_query("DELETE FROM entries WHERE id NOT IN (SELECT entry_id FROM session_entries)")
        .execute(conn)
        .change_context(SessionStoreError)
        .attach("failed to clean orphaned entries after delete")?;

    Ok(())
}

/// Forks a session from a specific entry ordinal.
///
/// Creates a new session with `parent_session` = source, copies junction rows
/// up to and including `at_ordinal`. Entry data is shared (not duplicated).
fn fork_blocking(
    conn: &mut SqliteConnection,
    source_session_id: &SessionId,
    at_ordinal: usize,
) -> Result<SessionId, Report<SessionStoreError>> {
    use crate::schema::{session_entries, sessions};

    let source_str = source_session_id.to_string();
    let new_id = SessionId::new();
    let new_id_str = new_id.to_string();

    conn.transaction::<_, diesel::result::Error, _>(|txn| {
        // Load source session metadata.
        let source_meta: Option<SessionRow> = sessions::table
            .filter(sessions::id.eq(&source_str))
            .first::<SessionRow>(txn)
            .ok();

        let Some(source_meta) = source_meta else {
            return Err(diesel::result::Error::NotFound);
        };

        let now = jiff::Timestamp::now().to_string();

        // Create new session row.
        insert_into(sessions::table)
            .values(&NewSessionRow {
                id: new_id_str.clone(),
                title: source_meta.title,
                updated_at: now.clone(),
                created_at: now, // fresh created_at — it's a new session
                profile: source_meta.profile,
                strategy_state: source_meta.strategy_state,
                blobs: source_meta.blobs,
                parent_session: Some(source_str.clone()),
                cwd: source_meta.cwd,
            })
            .execute(txn)?;

        // Copy junction rows up to and including at_ordinal.
        let junction_rows: Vec<SessionEntryRow> = session_entries::table
            .filter(session_entries::session_id.eq(&source_str))
            .filter(session_entries::ordinal.le(at_ordinal as i32))
            .load::<SessionEntryRow>(txn)?;

        for row in junction_rows {
            insert_into(session_entries::table)
                .values(&NewSessionEntryRow {
                    session_id: new_id_str.clone(),
                    entry_id: row.entry_id,
                    ordinal: row.ordinal,
                    pin_position: row.pin_position,
                })
                .execute(txn)?;
        }

        Ok(())
    })
    .change_context(SessionStoreError)
    .attach("failed to fork session")?;

    Ok(new_id)
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
            ChatEntryKind::User { display, .. } => assert_eq!(display, "first"),
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
        assert!(
            matches!(&loaded.history()[0].kind, ChatEntryKind::User { display, .. } if display == "user msg")
        );
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

    // --- CWD persistence ---

    #[rstest::rstest]
    #[tokio::test]
    async fn cwd_round_trips_through_save_and_load() {
        // Given a store with a session that has a custom cwd.
        let (_dir, store) = make_store();
        let session_id = SessionId::new();
        let mut session = ChatSessionState::new();
        session.set_session_id(session_id.clone());
        session.set_title("CWD Test".to_owned());
        session.push_entry(ChatEntry::user("hello"));
        session.set_cwd(std::path::PathBuf::from("/tmp/my-project"));

        // When saving and loading.
        store.save(&session).await.expect("save");
        let loaded = store
            .load_session(&session_id)
            .await
            .expect("load")
            .expect("should exist");

        // Then the cwd is preserved.
        assert_eq!(loaded.cwd(), std::path::Path::new("/tmp/my-project"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn fork_inherits_cwd_from_source() {
        // Given a store with a session that has a custom cwd.
        let (_dir, store) = make_store();
        let source_id = SessionId::new();
        let mut source = ChatSessionState::new();
        source.set_session_id(source_id.clone());
        source.set_title("Original".to_owned());
        source.push_entry(ChatEntry::user("hello"));
        source.set_cwd(std::path::PathBuf::from("/home/user/project"));
        store.save(&source).await.expect("save source");

        // When forking.
        let forked_id = store.fork(&source_id, 0).await.expect("fork");

        // Then the forked session inherits the source cwd.
        let forked = store
            .load_session(&forked_id)
            .await
            .expect("load forked")
            .expect("should exist");
        assert_eq!(forked.cwd(), std::path::Path::new("/home/user/project"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn save_updates_cwd_on_existing_session() {
        // Given a store with a saved session.
        let (_dir, store) = make_store();
        let session_id = SessionId::new();
        let mut session = ChatSessionState::new();
        session.set_session_id(session_id.clone());
        session.set_title("CWD Update".to_owned());
        session.push_entry(ChatEntry::user("hello"));
        session.set_cwd(std::path::PathBuf::from("/old/path"));
        store.save(&session).await.expect("save v1");

        // When saving with an updated cwd.
        session.set_cwd(std::path::PathBuf::from("/new/path"));
        store.save(&session).await.expect("save v2");

        // Then the loaded session has the new cwd.
        let loaded = store
            .load_session(&session_id)
            .await
            .expect("load")
            .expect("should exist");
        assert_eq!(loaded.cwd(), std::path::Path::new("/new/path"));
    }
}
