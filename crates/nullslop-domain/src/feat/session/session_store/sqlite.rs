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

use std::path::Path;

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

use super::migrator;
use super::{SessionStore, SessionStoreError};

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
        Self::build_pool(&dir, &PoolConfig::default())
    }

    /// Creates a store at an explicit directory (for testing).
    #[must_use]
    pub fn new_in(dir: &Path) -> Self {
        Self::build_pool(dir, &PoolConfig::default())
    }

    /// Creates a store at an explicit directory with custom pool configuration.
    #[must_use]
    pub fn new_with_config(dir: &Path, config: &PoolConfig) -> Self {
        Self::build_pool(dir, config)
    }

    /// Builds the connection pool.
    ///
    /// Creates the directory if needed, runs embedded migrations once on a
    /// bootstrap connection, then builds the pool. Each pooled connection gets
    /// WAL and foreign key pragmas set via the connection customizer.
    #[expect(clippy::expect_used, reason = "pool creation failures are fatal")]
    fn build_pool(dir: &Path, config: &PoolConfig) -> Self {
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
            migrator::run_migrations(&mut conn);
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

impl Default for SqliteSessionStore {
    fn default() -> Self {
        Self::new()
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
    /// Diesel `Queryable` requires this field to match the `session_entries.session_id`
    /// column returned by `SELECT *`. The Rust code already knows the session ID from
    /// the query filter, so this field is never read directly.
    #[expect(
        dead_code,
        reason = "required by Diesel Queryable derive to match SELECT * columns"
    )]
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
    /// Diesel `Queryable` requires this field to match the `token_ledger.id`
    /// column returned by `SELECT *`. The auto-increment PK is not used in Rust code.
    #[expect(
        dead_code,
        reason = "required by Diesel Queryable derive to match SELECT * columns"
    )]
    id: Option<i32>,
    /// Diesel `Queryable` requires this field to match the `token_ledger.session_id`
    /// column returned by `SELECT *`. The Rust code already knows the session ID from
    /// the query filter, so this field is never read directly.
    #[expect(
        dead_code,
        reason = "required by Diesel Queryable derive to match SELECT * columns"
    )]
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
                    lifecycle_name: _lifecycle_name,
                    lifecycle_args: _lifecycle_args,
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
            parent_session: parent_session
                .as_ref()
                .map(std::string::ToString::to_string),
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
                lifecycle_name: None,
                lifecycle_args: Vec::new(),
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
        // Info entries are runtime-only UI hints — skip them during persistence.
        for (ordinal, entry) in session
            .history()
            .iter()
            .enumerate()
            .filter(|(_, e)| !matches!(e.kind, crate::protocol::ChatEntryKind::Info(_)))
        {
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
    use crate::schema::sessions::dsl::sessions;

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
