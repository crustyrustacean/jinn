//! SQLite-backed session store implementation.
//!
//! Stores session data in normalized tables with a junction table for entries.
//! This eliminates duplication - each chat entry is stored once and shared
//! across sessions. The junction table enables fork support by copying only
//! small junction rows, not entry data.
//!
//! Uses Diesel's type-safe query DSL for compile-time column verification
//! against the generated schema. All queries are checked at compile time -
//! if a column is added to a migration but missing from an INSERT or SELECT,
//! the code will not compile.

use std::collections::HashMap;
use std::path::Path;

use async_trait::async_trait;
use diesel::insert_into;
use diesel::prelude::*;
use diesel::r2d2::{self as diesel_r2d2, CustomizeConnection, Pool};
use diesel::sql_query;
use diesel::upsert::excluded;
use error_stack::{Report, ResultExt as _};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use tokio::task::spawn_blocking;

use crate::common::app_info::APP_NAME;

use crate::feat::session::SessionUi;
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryKind};
use crate::feat::session::chat_history::ChatHistory;
use crate::feat::session::chat_session::{
    ChatSessionState, LifecycleScriptState, SessionCore, SessionCoreEphemeral, SessionState,
};
use crate::feat::session::profile::SessionProfile;
use crate::feat::session::session_summary::SessionSummary;
use crate::feat::session::token_stats::TokenRecord;
use crate::protocol::{ChatEntryId, ContextOverride, SessionId};

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
    /// Uses `dirs::data_dir()` → `jinn/sessions.db` on Linux.
    /// The database file is created on first access. Migrations are run
    /// once during pool initialization.
    ///
    /// # Errors
    ///
    /// Returns an error if the platform data directory cannot be determined,
    /// directory creation fails, or pool creation fails.
    pub fn new() -> Result<Self, Report<SessionStoreError>> {
        let dir = dirs::data_dir()
            .ok_or_else(|| {
                Report::new(SessionStoreError).attach("platform data directory not available")
            })?
            .join(APP_NAME);
        Self::build_pool(&dir, &PoolConfig::default())
    }

    /// Creates a store at an explicit directory (for testing).
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the database pool cannot be built.
    pub fn new_in(dir: &Path) -> Result<Self, Report<SessionStoreError>> {
        Self::build_pool(dir, &PoolConfig::default())
    }

    /// Creates a store at an explicit directory with custom pool configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the database pool cannot be built.
    pub fn new_with_config(
        dir: &Path,
        config: &PoolConfig,
    ) -> Result<Self, Report<SessionStoreError>> {
        Self::build_pool(dir, config)
    }

    /// Opens or creates a database at the exact file path.
    ///
    /// Creates parent directories if they don't exist. The database file
    /// is created by SQLite on first write (during migration).
    ///
    /// # Errors
    ///
    /// Returns an error if the parent directory cannot be created or the database pool cannot be built.
    pub fn open_or_create(file_path: &Path) -> Result<Self, Report<SessionStoreError>> {
        if let Some(parent) = file_path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            std::fs::create_dir_all(parent)
                .change_context(SessionStoreError)
                .attach("failed to create database directory")?;
        }
        Self::connect_at(file_path, &PoolConfig::default())
    }

    /// Builds the connection pool at a directory (appends `sessions.db`).
    fn build_pool(dir: &Path, config: &PoolConfig) -> Result<Self, Report<SessionStoreError>> {
        if !dir.exists() {
            std::fs::create_dir_all(dir)
                .change_context(SessionStoreError)
                .attach("failed to create session directory")?;
        }
        Self::connect_at(&dir.join(FILE_NAME), config)
    }

    /// Connects to the database at an exact file path and builds the pool.
    fn connect_at(
        file_path: &Path,
        config: &PoolConfig,
    ) -> Result<Self, Report<SessionStoreError>> {
        let database_url = file_path.to_string_lossy().to_string();

        // Run migrations once on a bootstrap connection before building the pool.
        // This ensures the schema is ready before any pooled connections are created.
        {
            let mut conn = SqliteConnection::establish(&database_url)
                .change_context(SessionStoreError)
                .attach("failed to open database for migration")?;
            diesel::sql_query("PRAGMA journal_mode=WAL")
                .execute(&mut conn)
                .change_context(SessionStoreError)
                .attach("failed to set WAL pragma")?;
            diesel::sql_query("PRAGMA foreign_keys=ON")
                .execute(&mut conn)
                .change_context(SessionStoreError)
                .attach("failed to set foreign_keys pragma")?;
            diesel::sql_query("PRAGMA busy_timeout=5000")
                .execute(&mut conn)
                .change_context(SessionStoreError)
                .attach("failed to set busy_timeout pragma")?;
            migrator::run_migrations(&mut conn)?;
        }

        let manager = diesel_r2d2::ConnectionManager::<SqliteConnection>::new(&database_url);
        let pool = Pool::builder()
            .max_size(config.max_size)
            .connection_customizer(Box::new(SqliteConnectionCustomizer))
            .build(manager)
            .change_context(SessionStoreError)
            .attach("failed to create connection pool")?;

        Ok(Self { pool })
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
        diesel::sql_query("PRAGMA busy_timeout=5000")
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
        let pool = self.pool.clone();
        let session = session.clone();
        spawn_blocking(move || {
            let mut conn = pool
                .get()
                .change_context(SessionStoreError)
                .attach("failed to acquire connection from pool")?;
            save_blocking(&mut conn, &session)
        })
        .await
        .change_context(SessionStoreError)
        .attach("spawn_blocking panicked during save")?
    }

    async fn load_summaries(&self) -> Result<Vec<SessionSummary>, Report<SessionStoreError>> {
        let pool = self.pool.clone();
        spawn_blocking(move || {
            let mut conn = pool
                .get()
                .change_context(SessionStoreError)
                .attach("failed to acquire connection from pool")?;
            load_summaries_blocking(&mut conn)
        })
        .await
        .change_context(SessionStoreError)
        .attach("spawn_blocking panicked during load_summaries")?
    }

    async fn load_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<ChatSessionState>, Report<SessionStoreError>> {
        let pool = self.pool.clone();
        let session_id = session_id.clone();
        spawn_blocking(move || {
            let mut conn = pool
                .get()
                .change_context(SessionStoreError)
                .attach("failed to acquire connection from pool")?;
            load_session_blocking(&mut conn, &session_id)
        })
        .await
        .change_context(SessionStoreError)
        .attach("spawn_blocking panicked during load_session")?
    }

    async fn delete(&self, session_id: &SessionId) -> Result<(), Report<SessionStoreError>> {
        let pool = self.pool.clone();
        let session_id = session_id.clone();
        spawn_blocking(move || {
            let mut conn = pool
                .get()
                .change_context(SessionStoreError)
                .attach("failed to acquire connection from pool")?;
            delete_blocking(&mut conn, &session_id)
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
        let pool = self.pool.clone();
        let source_session_id = source_session_id.clone();
        spawn_blocking(move || {
            let mut conn = pool
                .get()
                .change_context(SessionStoreError)
                .attach("failed to acquire connection from pool")?;
            fork_blocking(&mut conn, &source_session_id, at_ordinal)
        })
        .await
        .change_context(SessionStoreError)
        .attach("spawn_blocking panicked during fork")?
    }

    async fn set_archived(
        &self,
        session_id: &SessionId,
        archived: bool,
    ) -> Result<(), Report<SessionStoreError>> {
        let pool = self.pool.clone();
        let session_id = session_id.clone();
        spawn_blocking(move || {
            let mut conn = pool
                .get()
                .change_context(SessionStoreError)
                .attach("failed to acquire connection from pool")?;
            set_archived_blocking(&mut conn, &session_id, archived)
        })
        .await
        .change_context(SessionStoreError)
        .attach("spawn_blocking panicked during set_archived")?
    }

    async fn load_unarchived_summaries(
        &self,
    ) -> Result<Vec<SessionSummary>, Report<SessionStoreError>> {
        let pool = self.pool.clone();
        spawn_blocking(move || {
            let mut conn = pool
                .get()
                .change_context(SessionStoreError)
                .attach("failed to acquire connection from pool")?;
            load_unarchived_summaries_blocking(&mut conn)
        })
        .await
        .change_context(SessionStoreError)
        .attach("spawn_blocking panicked during load_unarchived_summaries")?
    }

    async fn shutdown(&self) -> Result<(), Report<SessionStoreError>> {
        let pool = self.pool.clone();
        spawn_blocking(move || {
            let mut conn = pool
                .get()
                .change_context(SessionStoreError)
                .attach("failed to acquire connection from pool")?;
            shutdown_blocking(&mut conn);
            Ok(())
        })
        .await
        .change_context(SessionStoreError)
        .attach("spawn_blocking panicked during shutdown")?
    }
}

// ── Diesel model structs ─────────────────────────────────────────────────

/// Reading model for the `sessions` table.
///
/// Uses `QueryableByName` to map columns by name rather than position.
/// This bypasses Diesel's tuple-size limit (10 fields) which would
/// otherwise prevent compiling with 11 columns.
#[derive(QueryableByName)]
#[diesel(table_name = crate::schema::sessions)]
struct SessionRow {
    id: Option<String>,
    title: Option<String>,
    updated_at: String,
    profile: String,

    blobs: String,
    parent_session: Option<String>,
    cwd: String,
    created_at: String,
    lifecycle_name: Option<String>,
    lifecycle_args: String,
    archived: bool,
    lifecycle_script_state: String,
    metadata: Option<String>,
    is_workflow: bool,
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

    blobs: String,
    parent_session: Option<String>,
    cwd: String,
    lifecycle_name: Option<String>,
    lifecycle_args: String,
    archived: bool,
    lifecycle_script_state: String,
    metadata: Option<String>,
    is_workflow: bool,
}

/// Reading model for the `entries` table.
#[derive(Queryable)]
#[diesel(table_name = crate::schema::entries)]
struct EntryRow {
    id: Option<String>,
    timestamp: String,
    kind: String,
    context_history: String,
}

/// Insert model for the `entries` table.
#[derive(Insertable)]
#[diesel(table_name = crate::schema::entries)]
struct NewEntryRow {
    id: String,
    timestamp: String,
    kind: String,
    context_history: String,
}

/// Reading model for the `session_history` table.
#[derive(Queryable)]
#[diesel(table_name = crate::schema::session_history)]
struct SessionEntryRow {
    /// Diesel `Queryable` requires this field to match the `session_history.session_id`
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
    /// Legacy column kept for backward compatibility. New code uses `context_override`.
    ignored: bool,
    context_override: String,
}

/// Insert model for the `session_history` table.
#[derive(Insertable)]
#[diesel(table_name = crate::schema::session_history)]
struct NewSessionEntryRow {
    session_id: String,
    entry_id: String,
    ordinal: i32,
    pin_position: Option<String>,
    ignored: bool,
    context_override: String,
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
    cost: Option<f64>,
}

/// Insert model for the `token_ledger` table.
#[derive(Insertable)]
#[diesel(table_name = crate::schema::token_ledger)]
struct NewTokenLedgerRow {
    session_id: String,
    timestamp: String,
    tokens_sent: i32,
    tokens_received: i32,
    cost: Option<f64>,
}

// ── Conversions ──────────────────────────────────────────────────────────

// ── PersistableCore - JSON blob for session metadata ─────────────────────

/// A subset of [`SessionCore`] fields suitable for JSON blob persistence.
///
/// Excludes `history`, `token_ledger`, and `ephemeral` which are stored in
/// normalized tables or are runtime-only. This blob acts as a snapshot that
/// can be deserialized back into a full `SessionCore` with defaults for the
/// excluded fields.
#[derive(Serialize, Deserialize)]
struct PersistableCore {
    session_id: SessionId,
    title: Option<String>,
    updated_at: jiff::Timestamp,
    created_at: jiff::Timestamp,
    profile: SessionProfile,
    cwd: std::path::PathBuf,
    parent_session: Option<SessionId>,

    blobs: HashMap<String, JsonValue>,
    lifecycle_name: Option<String>,
    lifecycle_args: Vec<String>,
    lifecycle_script_state: LifecycleScriptState,
    /// Phased task list for agent session planning.
    /// OWNER: tools-actor (mutated by task list tools).
    #[serde(default)]
    task_list: crate::feat::todo_list::TaskList,
    /// Attached plugins - persistent per-session plugin attachments.
    /// OWNER: plugin-dispatch-actor (attach/detach/toggle).
    #[serde(default)]
    attached_plugins: Vec<crate::feat::attached_plugin::AttachedPlugin>,
}

impl From<&SessionCore> for PersistableCore {
    fn from(core: &SessionCore) -> Self {
        Self {
            session_id: core.session_id.clone(),
            title: core.title.clone(),
            updated_at: core.updated_at,
            created_at: core.created_at,
            profile: core.profile.clone(),
            cwd: core.cwd.clone(),
            parent_session: core.parent_session.clone(),

            blobs: core.blobs.clone(),
            lifecycle_name: core.lifecycle_name.clone(),
            lifecycle_args: core.lifecycle_args.clone(),
            lifecycle_script_state: core.lifecycle_script_state,
            task_list: core.task_list.clone(),
            attached_plugins: core.attached_plugins.clone(),
        }
    }
}

impl From<PersistableCore> for SessionCore {
    fn from(core: PersistableCore) -> Self {
        Self {
            session_id: core.session_id,
            title: core.title,
            updated_at: core.updated_at,
            created_at: core.created_at,
            history: ChatHistory::new(),
            profile: core.profile,
            cwd: core.cwd,
            token_ledger: vec![],
            parent_session: core.parent_session,

            blobs: core.blobs,
            lifecycle_name: core.lifecycle_name,
            lifecycle_args: core.lifecycle_args,
            session_state: SessionState::Loaded, // overridden by TryFrom<SessionLoadContext> from archived column
            lifecycle_script_state: core.lifecycle_script_state,
            ephemeral: SessionCoreEphemeral::default(),
            is_workflow: false,       // set from DB column after deserialization
            workflow_overrides: None, // runtime-only, never persisted
            has_interacted: false, // restored sessions get mark_interacted() in handle_session_load_completed
            task_list: core.task_list,
            attached_plugins: core.attached_plugins,
        }
    }
}

impl TryFrom<&ChatSessionState> for NewSessionRow {
    type Error = Report<SessionStoreError>;

    #[deny(unused_variables)]
    fn try_from(session: &ChatSessionState) -> Result<Self, Self::Error> {
        // Exhaustive destructuring - adding a field to SessionCore
        // without updating this pattern is a compile error.
        let ChatSessionState {
            core:
                SessionCore {
                    session_id,
                    title,
                    updated_at,
                    created_at,
                    history: _history, // persisted via entries + session_history tables below
                    profile,
                    cwd,
                    token_ledger: _ledger, // persisted via token_ledger table below
                    parent_session,

                    blobs,
                    lifecycle_name,
                    lifecycle_args,
                    ephemeral: _ephemeral, // runtime-only state, not persisted
                    session_state,
                    lifecycle_script_state,
                    is_workflow,
                    workflow_overrides: _workflow_overrides, // runtime-only, not persisted
                    has_interacted: _has_interacted, // deserialized from DB, restored by handle_session_load_completed
                    task_list: _task_list, // included in metadata blob via PersistableCore
                    attached_plugins: _attached_plugins, // included in metadata blob via PersistableCore
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

            blobs: serde_json::to_string(blobs)
                .change_context(SessionStoreError)
                .attach("failed to serialize blobs")?,
            parent_session: parent_session
                .as_ref()
                .map(std::string::ToString::to_string),
            cwd: cwd.to_string_lossy().to_string(),
            lifecycle_name: lifecycle_name.clone(),
            lifecycle_args: serde_json::to_string(lifecycle_args)
                .change_context(SessionStoreError)
                .attach("failed to serialize lifecycle_args")?,
            archived: *session_state == SessionState::Archived,
            lifecycle_script_state: serde_json::to_string(&lifecycle_script_state)
                .change_context(SessionStoreError)
                .attach("failed to serialize lifecycle_script_state")?,
            metadata: serde_json::to_string(&PersistableCore::from(&session.core))
                .change_context(SessionStoreError)
                .attach("failed to serialize metadata")?
                .into(),
            is_workflow: *is_workflow,
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
        // Exhaustive destructuring of SessionRow - adding a column to the
        // sessions table without updating this pattern is a compile error.
        let SessionRow {
            id,
            title,
            updated_at,
            created_at,
            profile,

            blobs,
            parent_session,
            cwd,
            lifecycle_name,
            lifecycle_args,
            archived,
            lifecycle_script_state,
            metadata,
            is_workflow,
        } = ctx.row;

        // When a metadata JSON blob exists (v8+), deserialize it as the
        // authoritative source of truth for SessionCore fields, then overlay
        // the normalized-table data (entries, token_ledger).
        let mut core = if let Some(ref json) = metadata {
            let persistable: PersistableCore = serde_json::from_str(json)
                .change_context(SessionStoreError)
                .attach("failed to deserialize session metadata blob")?;
            SessionCore::from(persistable)
        } else {
            // Legacy path - reconstruct from individual columns (pre-v8 data).
            let profile = serde_json::from_str(&profile)
                .change_context(SessionStoreError)
                .attach("failed to deserialize profile")?;

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

            SessionCore {
                session_id: SessionId::from(id.unwrap_or_default()),
                title,
                updated_at,
                created_at,
                history: ChatHistory::new(),
                profile,
                cwd: std::path::PathBuf::from(cwd),
                token_ledger: vec![],
                parent_session: parent_session.map(SessionId::from),

                blobs,
                lifecycle_name,
                lifecycle_args: serde_json::from_str(&lifecycle_args).unwrap_or_default(),
                ephemeral: SessionCoreEphemeral::default(),
                session_state: SessionState::Loaded,
                lifecycle_script_state: serde_json::from_str(&lifecycle_script_state)
                    .unwrap_or_default(),
                is_workflow: false,

                workflow_overrides: None, // runtime-only, set later if needed
                has_interacted: false, // restored sessions get mark_interacted() in handle_session_load_completed
                task_list: crate::feat::todo_list::TaskList::default(), // no metadata blob available for legacy sessions
                attached_plugins: Vec::default(),
            }
        };

        // Single source of truth: is_workflow column → core.is_workflow.
        core.is_workflow = is_workflow;

        // Single source of truth: archived column → session_state.
        core.session_state = if archived {
            SessionState::Archived
        } else {
            SessionState::Loaded
        };

        // Overlay data from normalized tables (always loaded regardless of path).
        core.history = ChatHistory::from_vec(ctx.entries);
        core.token_ledger = ctx.ledger;

        // Build ChatSessionState with all fields explicitly set.
        Ok(ChatSessionState {
            core,
            ui: SessionUi::default(),
        })
    }
}

// ── Blocking implementations ─────────────────────────────────────────────

/// Saves a complete session in a single transaction.
///
/// Upserts session metadata, replaces all junction rows and token ledger rows,
/// and inserts any new entries. Orphaned-entry reaping is intentionally not done
/// here — it belongs in `delete_blocking`/`fork_blocking`, where the removing
/// session is known. A global cleanup in the save hot-path could wipe every
/// entry if `session_history` is transiently empty (e.g. mid-migration).
fn save_blocking(
    conn: &mut SqliteConnection,
    session: &ChatSessionState,
) -> Result<(), Report<SessionStoreError>> {
    let row = NewSessionRow::try_from(session)?;
    let session_id_str = row.id.clone();

    conn.transaction::<_, diesel::result::Error, _>(|txn| {
        use crate::schema::{entries, session_history, sessions, token_ledger};

        // Upsert session metadata.
        insert_into(sessions::table)
            .values(&row)
            .on_conflict(sessions::dsl::id)
            .do_update()
            .set((
                sessions::title.eq(excluded(sessions::title)),
                sessions::updated_at.eq(excluded(sessions::updated_at)),
                sessions::profile.eq(excluded(sessions::profile)),
                sessions::blobs.eq(excluded(sessions::blobs)),
                sessions::cwd.eq(excluded(sessions::cwd)),
                sessions::lifecycle_name.eq(excluded(sessions::lifecycle_name)),
                sessions::lifecycle_args.eq(excluded(sessions::lifecycle_args)),
                sessions::archived.eq(excluded(sessions::archived)),
                sessions::lifecycle_script_state.eq(excluded(sessions::lifecycle_script_state)),
                sessions::metadata.eq(excluded(sessions::metadata)),
                sessions::is_workflow.eq(excluded(sessions::is_workflow)),
            ))
            .execute(txn)?;

        // Delete existing junction rows and token ledger for this session.
        diesel::delete(
            session_history::table.filter(session_history::session_id.eq(&session_id_str)),
        )
        .execute(txn)?;

        diesel::delete(token_ledger::table.filter(token_ledger::session_id.eq(&session_id_str)))
            .execute(txn)?;

        // Insert entries and junction rows.
        // Transient entries are runtime-only UI hints - skip them during persistence.
        for (ordinal, entry) in session
            .history()
            .iter()
            .enumerate()
            .filter(|(_, e)| !matches!(e.kind, crate::protocol::ChatEntryKind::Transient(_)))
        {
            let entry_id_str = entry.id.to_string();
            let timestamp_str = entry.timestamp.to_string();
            let kind_json = serde_json::to_string(&entry.kind)
                .map_err(|e| diesel::result::Error::SerializationError(Box::new(e)))?;
            let pin_str = entry.pin_position.map(|p| p.to_string());

            // Serialize audit trail (empty array if no events recorded).
            let context_history_json = serde_json::to_string(&entry.context_history)
                .map_err(|e| diesel::result::Error::SerializationError(Box::new(e)))?;

            // Insert entry. On conflict (entry shared across sessions), update
            // context_history since it mutates after first insertion via
            // `apply_context_override`.
            let context_history_value = context_history_json.clone();
            insert_into(entries::table)
                .values(&NewEntryRow {
                    id: entry_id_str.clone(),
                    timestamp: timestamp_str,
                    kind: kind_json,
                    context_history: context_history_json,
                })
                .on_conflict(entries::dsl::id)
                .do_update()
                .set(entries::dsl::context_history.eq(context_history_value))
                .execute(txn)?;

            // Insert junction row.
            insert_into(session_history::table)
                .values(&NewSessionEntryRow {
                    session_id: session_id_str.clone(),
                    entry_id: entry_id_str,
                    ordinal: ordinal as i32,
                    pin_position: pin_str,
                    ignored: entry.ignored(),
                    context_override: serde_json::to_string(&entry.context_override())
                        .unwrap_or_else(|_| "\"default\"".to_owned()),
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
                    cost: record.cost,
                })
                .execute(txn)?;
        }

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
    let rows: Vec<SessionRow> = sql_query("SELECT * FROM sessions")
        .load(conn)
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
            session_state: if row.archived {
                SessionState::Archived
            } else {
                SessionState::Loaded
            },
            parent_session: row.parent_session.map(SessionId::from),
        })
        .collect();

    Ok(summaries)
}

/// Loads a full session by ID.
fn load_session_blocking(
    conn: &mut SqliteConnection,
    session_id: &SessionId,
) -> Result<Option<ChatSessionState>, Report<SessionStoreError>> {
    use crate::schema::{entries, session_history, token_ledger};

    let session_id_str = session_id.to_string();

    // Load session metadata.
    let meta: Option<SessionRow> = sql_query("SELECT * FROM sessions WHERE id = ?")
        .bind::<diesel::sql_types::Text, _>(&session_id_str)
        .get_result(conn)
        .ok();

    let Some(meta) = meta else {
        return Ok(None);
    };

    // Load entries via junction table, ordered by ordinal.
    let joined: Vec<(EntryRow, SessionEntryRow)> = entries::table
        .inner_join(session_history::table)
        .filter(session_history::session_id.eq(&session_id_str))
        .order(session_history::ordinal.asc())
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

            let row_id = entry.id.clone().unwrap_or_default();
            let row_timestamp = entry.timestamp.clone();
            let mut chat_entry = ChatEntry::new_with_kind(
                ChatEntryId::from(row_id),
                row_timestamp
                    .parse()
                    .unwrap_or_else(|_| jiff::Timestamp::now()),
                kind,
                pin_position,
            );
            // Restored from DB - no audit event recorded.
            let override_value: ContextOverride = serde_json::from_str(&junction.context_override)
                .unwrap_or_else(|e| {
                    tracing::warn!(
                        entry_id = %chat_entry.id.as_uuid(),
                        raw = %junction.context_override,
                        error = %e,
                        "failed to deserialize context_override, falling back to Default"
                    );
                    // Fallback: use legacy ignored column if context_override is corrupt
                    if junction.ignored {
                        ContextOverride::ForcedExclude
                    } else {
                        ContextOverride::Default
                    }
                });
            chat_entry.restore_context_override(override_value);

            // Restore audit trail. Empty array (default) loads as Vec::new().
            // Corrupt JSON falls back to empty with a warning.
            chat_entry.context_history = serde_json::from_str(&entry.context_history)
                .unwrap_or_else(|e| {
                    tracing::warn!(
                        entry_id = %chat_entry.id.as_uuid(),
                        raw = %entry.context_history,
                        error = %e,
                        "failed to deserialize context_history, falling back to empty"
                    );
                    Vec::new()
                });
            chat_entry
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
            cost: row.cost,
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
/// Deletes a session and reaps entries that became orphaned by this delete.
///
/// Cleanup is **scoped to this session's own former entries**: the session's
/// `entry_id`s are captured before the FK cascade removes its junction rows,
/// then only those candidates that no remaining session references are deleted.
/// A global orphan sweep is deliberately avoided — a transiently-empty global
/// `session_history` state can never cause mass reaping of other sessions' data.
fn delete_blocking(
    conn: &mut SqliteConnection,
    session_id: &SessionId,
) -> Result<(), Report<SessionStoreError>> {
    let session_id_str = session_id.to_string();

    conn.transaction::<_, diesel::result::Error, _>(|txn| {
        use crate::schema::{entries, session_history, sessions};

        // Capture this session's entry references before the FK cascade
        // removes them. These are the only candidates for reaping.
        let candidates: Vec<String> = session_history::table
            .filter(session_history::session_id.eq(&session_id_str))
            .select(session_history::entry_id)
            .distinct()
            .load(txn)?;

        // Delete the session. With FK=ON this cascades to remove this session's
        // session_history and token_ledger rows.
        diesel::delete(sessions::table.filter(sessions::id.eq(&session_id_str))).execute(txn)?;

        if candidates.is_empty() {
            return Ok(());
        }

        // After the cascade, session_history holds only OTHER sessions'
        // references. Reap a candidate only if no remaining session claims it.
        let still_referenced: Vec<String> = session_history::table
            .filter(session_history::entry_id.eq_any(&candidates))
            .select(session_history::entry_id)
            .distinct()
            .load(txn)?;
        let orphaned: Vec<String> = candidates
            .iter()
            .filter(|id| !still_referenced.contains(id))
            .cloned()
            .collect();

        if !orphaned.is_empty() {
            diesel::delete(entries::table.filter(entries::id.eq_any(&orphaned))).execute(txn)?;
        }

        Ok(())
    })
    .change_context(SessionStoreError)
    .attach("failed to delete session")?;

    Ok(())
}

/// Forks a session from a specific entry ordinal.
///
/// Creates a new session with `parent_session` = source, copies junction rows
/// up to and including `at_ordinal`. Entry data is shared (not duplicated).
/// Patches a metadata JSON blob for a forked session.
///
/// Overrides `parent_session`, `session_id`, `created_at`, and `updated_at`
/// so the forked session's metadata reflects its new identity.
/// Falls back to `None` if deserialization or re-serialization fails.
fn fork_metadata(
    source_metadata: Option<&String>,
    source_id_str: &str,
    new_id_str: &str,
) -> Option<String> {
    let json = source_metadata.as_ref()?;
    let mut core: PersistableCore = serde_json::from_str(json).ok()?;
    core.parent_session = Some(SessionId::from(source_id_str.to_owned()));
    core.session_id = SessionId::from(new_id_str.to_owned());
    core.created_at = jiff::Timestamp::now();
    core.updated_at = jiff::Timestamp::now();
    serde_json::to_string(&core).ok()
}

fn fork_blocking(
    conn: &mut SqliteConnection,
    source_session_id: &SessionId,
    at_ordinal: usize,
) -> Result<SessionId, Report<SessionStoreError>> {
    use crate::schema::{session_history, sessions};

    let source_str = source_session_id.to_string();
    let new_id = SessionId::new();
    let new_id_str = new_id.to_string();

    conn.transaction::<_, diesel::result::Error, _>(|txn| {
        // Load source session metadata.
        let source_meta: Option<SessionRow> = sql_query("SELECT * FROM sessions WHERE id = ?")
            .bind::<diesel::sql_types::Text, _>(&source_str)
            .get_result(txn)
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
                created_at: now, // fresh created_at - it's a new session
                profile: source_meta.profile,

                blobs: source_meta.blobs,
                parent_session: Some(source_str.clone()),
                cwd: source_meta.cwd,
                lifecycle_name: source_meta.lifecycle_name,
                lifecycle_args: source_meta.lifecycle_args,
                archived: false,
                lifecycle_script_state: source_meta.lifecycle_script_state,
                metadata: fork_metadata(source_meta.metadata.as_ref(), &source_str, &new_id_str),
                is_workflow: source_meta.is_workflow,
            })
            .execute(txn)?;

        // Copy junction rows up to and including at_ordinal.
        let junction_rows: Vec<SessionEntryRow> = session_history::table
            .filter(session_history::session_id.eq(&source_str))
            .filter(session_history::ordinal.le(at_ordinal as i32))
            .load::<SessionEntryRow>(txn)?;

        for row in junction_rows {
            insert_into(session_history::table)
                .values(&NewSessionEntryRow {
                    session_id: new_id_str.clone(),
                    entry_id: row.entry_id,
                    ordinal: row.ordinal,
                    pin_position: row.pin_position,
                    ignored: row.ignored,
                    context_override: row.context_override,
                })
                .execute(txn)?;
        }

        Ok(())
    })
    .change_context(SessionStoreError)
    .attach("failed to fork session")?;

    Ok(new_id)
}

/// Sets the `archived` flag for a session.
fn set_archived_blocking(
    conn: &mut SqliteConnection,
    session_id: &SessionId,
    archived: bool,
) -> Result<(), Report<SessionStoreError>> {
    let session_id_str = session_id.to_string();
    sql_query("UPDATE sessions SET archived = ? WHERE id = ?")
        .bind::<diesel::sql_types::Bool, _>(archived)
        .bind::<diesel::sql_types::Text, _>(&session_id_str)
        .execute(conn)
        .change_context(SessionStoreError)
        .attach("failed to set archived flag")?;

    Ok(())
}

/// Loads summaries for all unarchived sessions.
fn load_unarchived_summaries_blocking(
    conn: &mut SqliteConnection,
) -> Result<Vec<SessionSummary>, Report<SessionStoreError>> {
    let rows: Vec<SessionRow> = sql_query("SELECT * FROM sessions WHERE archived = FALSE")
        .load(conn)
        .change_context(SessionStoreError)
        .attach("failed to query unarchived summaries")?;

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
            session_state: if row.archived {
                SessionState::Archived
            } else {
                SessionState::Loaded
            },
            parent_session: row.parent_session.map(SessionId::from),
        })
        .collect();

    Ok(summaries)
}

/// Non-destructive no-op called during shutdown.
///
/// Previously this deleted "empty" unarchived sessions (no `session_history`
/// rows) and orphaned entries. That heuristic was a data-destruction hazard:
/// with `PRAGMA foreign_keys=ON` the session delete cascaded through
/// `token_ledger`, and it fired against any transiently-empty junction table
/// state (e.g. after a migration). The migration framework now runs with
/// foreign keys disabled, and orphan reaping lives only in `delete`/`fork`
/// where the removing session is known, so this hook no longer needs to delete
/// anything. Retained as a no-op to preserve the shutdown plumbing for any
/// future non-destructive flush work.
fn shutdown_blocking(_conn: &mut SqliteConnection) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_or_create_creates_parent_dirs_and_database() {
        // Given a nested nonexistent path.
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("a").join("b").join("c").join("test.db");

        // When opening the database.
        let store = SqliteSessionStore::open_or_create(&db_path).expect("open_or_create");

        // Then the parent directories were created.
        assert!(db_path.parent().unwrap().exists());
        // And the database is usable.
        let summaries = store.load_summaries().await;
        assert!(summaries.is_ok());
    }

    #[tokio::test]
    async fn open_or_create_opens_existing_database() {
        // Given a database created with new_in.
        let dir = tempfile::tempdir().expect("temp dir");
        let store = SqliteSessionStore::new_in(dir.path()).expect("new_in");
        // Verify it's usable.
        assert!(store.load_summaries().await.is_ok());

        // When opening the exact file path with open_or_create.
        let db_file = dir.path().join("sessions.db");
        let store2 = SqliteSessionStore::open_or_create(&db_file).expect("open_or_create");

        // Then the database is usable.
        assert!(store2.load_summaries().await.is_ok());
    }

    #[tokio::test]
    async fn open_or_create_works_with_bare_filename() {
        // Given a bare filename (no directory component).
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("test.db");

        // When opening the database.
        let store = SqliteSessionStore::open_or_create(&db_path).expect("open_or_create");

        // Then the database is usable.
        assert!(store.load_summaries().await.is_ok());
    }

    // ── Round-trip persistence tests ───────────────────────────────────

    /// Helper: create a fresh in-memory store for testing.
    fn fresh_store() -> SqliteSessionStore {
        let dir = tempfile::tempdir().expect("temp dir");
        SqliteSessionStore::new_in(dir.path()).expect("new_in")
    }

    /// Helper: create a minimal session with one user entry.
    fn make_session() -> ChatSessionState {
        let mut session = ChatSessionState::new();
        session.push_entry(ChatEntry::user("hello world"));
        session
    }

    #[tokio::test]
    async fn save_then_load_summaries_returns_saved_session() {
        // Given a store with a saved session.
        let store = fresh_store();
        let session = make_session();
        let id = session.session_id().clone();
        store.save(&session).await.expect("save");

        // When loading summaries.
        let summaries = store.load_summaries().await.expect("load_summaries");

        // Then the saved session appears in the list.
        assert_eq!(summaries.len(), 1, "should have exactly 1 summary");
        assert_eq!(summaries[0].session_id, id);
    }

    #[tokio::test]
    async fn delete_removes_session() {
        // Given a store with a saved session.
        let store = fresh_store();
        let session = make_session();
        let id = session.session_id().clone();
        store.save(&session).await.expect("save");

        // When deleting the session.
        store.delete(&id).await.expect("delete");

        // Then the session is gone from summaries.
        let summaries = store.load_summaries().await.expect("load_summaries");
        assert!(summaries.is_empty(), "session should be deleted");
    }

    #[tokio::test]
    async fn fork_creates_new_session_with_entries() {
        // Given a store with a saved session.
        let store = fresh_store();
        let session = make_session();
        let source_id = session.session_id().clone();
        store.save(&session).await.expect("save");

        // When forking the session.
        let fork_id = store.fork(&source_id, 1).await.expect("fork");

        // Then the fork has a different ID.
        assert_ne!(fork_id, source_id, "forked session should have a new ID");

        // And the forked session can be loaded.
        let loaded = store
            .load_session(&fork_id)
            .await
            .expect("load")
            .expect("forked session should exist");
        assert_eq!(loaded.history().len(), 1, "fork should copy entries");
    }

    #[tokio::test]
    async fn set_archived_removes_from_unarchived_summaries() {
        // Given a store with a saved session.
        let store = fresh_store();
        let session = make_session();
        let id = session.session_id().clone();
        store.save(&session).await.expect("save");

        // Verify the session appears in unarchived summaries BEFORE archiving.
        let unarchived_before = store
            .load_unarchived_summaries()
            .await
            .expect("load_unarchived");
        assert_eq!(
            unarchived_before.len(),
            1,
            "session should appear in unarchived before archiving"
        );

        // When archiving the session.
        store.set_archived(&id, true).await.expect("set_archived");

        // Then it disappears from unarchived summaries.
        let unarchived_after = store
            .load_unarchived_summaries()
            .await
            .expect("load_unarchived");
        assert!(
            unarchived_after.is_empty(),
            "archived session should not appear"
        );

        // And it still appears in all summaries.
        let all = store.load_summaries().await.expect("load_summaries");
        assert_eq!(
            all.len(),
            1,
            "archived session should still be in all summaries"
        );
    }

    #[tokio::test]
    async fn archived_flag_persists_across_save_and_load() {
        // Given a store.
        let store = fresh_store();

        // When saving an archived session.
        let mut session = make_session();
        session.set_session_state(SessionState::Archived);
        let id = session.session_id().clone();
        store.save(&session).await.expect("save");

        // Then loading it back shows it as archived.
        let loaded = store
            .load_session(&id)
            .await
            .expect("load")
            .expect("session should exist");
        assert_eq!(
            loaded.session_state(),
            SessionState::Archived,
            "loaded session should be archived"
        );

        // And saving an active session loads as active.
        let mut session2 = make_session();
        session2.set_session_state(SessionState::Loaded);
        let id2 = session2.session_id().clone();
        store.save(&session2).await.expect("save");
        let loaded2 = store
            .load_session(&id2)
            .await
            .expect("load")
            .expect("session should exist");
        assert_eq!(
            loaded2.session_state(),
            SessionState::Loaded,
            "loaded session should be active"
        );
    }

    #[tokio::test]
    async fn fork_preserves_parent_metadata() {
        // Given a store with a saved session.
        let store = fresh_store();
        let session = make_session();
        let source_id = session.session_id().clone();
        let source_created = *session.created_at();
        store.save(&session).await.expect("save");

        // When forking.
        let fork_id = store.fork(&source_id, 1).await.expect("fork");

        // Then the forked session's parent points to the source.
        let loaded = store
            .load_session(&fork_id)
            .await
            .expect("load")
            .expect("forked session should exist");
        assert_eq!(
            loaded.parent_session().as_ref(),
            Some(&source_id),
            "fork should have parent_session set"
        );

        // And the forked session has a different created_at (fork_metadata patches timestamps).
        assert_ne!(
            loaded.created_at(),
            &source_created,
            "fork should update created_at via fork_metadata"
        );
    }

    #[tokio::test]
    async fn shutdown_preserves_all_sessions() {
        // Given a store with an empty session (no history entries) and a full one.
        let store = fresh_store();
        let empty_session = ChatSessionState::new();
        let empty_id = empty_session.session_id().clone();
        store.save(&empty_session).await.expect("save empty");

        let full_session = make_session();
        let full_id = full_session.session_id().clone();
        store.save(&full_session).await.expect("save full");

        // When shutting down.
        store.shutdown().await.expect("shutdown");

        // Then both sessions survive — shutdown is a non-destructive no-op.
        let after = store
            .load_summaries()
            .await
            .expect("load_summaries after shutdown");
        let ids: Vec<_> = after.iter().map(|s| &s.session_id).collect();
        assert!(
            ids.contains(&&full_id),
            "non-empty session should survive shutdown"
        );
        assert!(
            ids.contains(&&empty_id),
            "empty session should survive shutdown (no destructive cleanup)"
        );
    }

    #[tokio::test]
    async fn steering_buffer_is_not_persisted_across_save_and_load() {
        // Given a session with two steering fragments in its buffer.
        let store = fresh_store();
        let mut session = make_session();
        session
            .steering_buffer_mut()
            .push_fragment("first".to_owned());
        session
            .steering_buffer_mut()
            .push_fragment("second".to_owned());
        assert_eq!(
            session.steering_buffer().len(),
            2,
            "pre-save buffer should hold both fragments"
        );

        // When saving the session.
        let id = session.session_id().clone();
        store.save(&session).await.expect("save");

        // And loading it back.
        let loaded = store
            .load_session(&id)
            .await
            .expect("load")
            .expect("session should exist after save");

        // Then the steering buffer is dropped on reload.
        assert!(
            loaded.steering_buffer().is_empty(),
            "steering buffer must not be persisted; should be empty after load"
        );
        assert_eq!(
            loaded.steering_buffer().len(),
            0,
            "loaded buffer len should be zero"
        );
    }

    // ── context_history persistence ─────────────────────────────────

    #[tokio::test]
    async fn context_history_survives_persistence_round_trip() {
        // Given an entry pre-populated with two audit events.
        use crate::feat::session::chat_entry::ChangeSource;

        let store = fresh_store();
        let mut entry = ChatEntry::user("hello world");
        entry.apply_context_override(ContextOverride::ForcedExclude, ChangeSource::User);
        entry.apply_context_override(
            ContextOverride::ForcedInclude,
            ChangeSource::Worker {
                name: "compactor".to_owned(),
            },
        );
        let entry_id = entry.id.clone();
        let mut session = ChatSessionState::new();
        session.push_entry(entry);
        store.save(&session).await.expect("save");

        // When reloading the session.
        let loaded = store
            .load_session(session.session_id())
            .await
            .expect("load")
            .expect("session should exist");

        // Then the audit trail is preserved with both events in order.
        let loaded_entry = loaded
            .history()
            .iter()
            .find(|e| e.id == entry_id)
            .expect("entry should exist");
        assert_eq!(
            loaded_entry.context_history.len(),
            2,
            "both audit events should survive the round trip"
        );

        // And the first event records Default -> ForcedExclude by the user.
        assert_eq!(
            loaded_entry.context_history[0].from,
            ContextOverride::Default
        );
        assert_eq!(
            loaded_entry.context_history[0].to,
            ContextOverride::ForcedExclude
        );
        assert!(matches!(
            &loaded_entry.context_history[0].source,
            ChangeSource::User
        ));

        // And the second event records ForcedExclude -> ForcedInclude by the compactor.
        assert_eq!(
            loaded_entry.context_history[1].from,
            ContextOverride::ForcedExclude
        );
        assert_eq!(
            loaded_entry.context_history[1].to,
            ContextOverride::ForcedInclude
        );
        assert!(matches!(
            &loaded_entry.context_history[1].source,
            ChangeSource::Worker { name } if name == "compactor"
        ));
    }

    #[tokio::test]
    async fn context_history_loads_as_empty_for_entries_with_default_value() {
        // Given a freshly-saved session with no audit events.
        let store = fresh_store();
        let session = make_session();
        let id = session.session_id().clone();
        store.save(&session).await.expect("save");

        // When reloading.
        let loaded = store
            .load_session(&id)
            .await
            .expect("load")
            .expect("session should exist");

        // Then the entry has an empty context_history (not an error).
        assert!(
            loaded.history()[0].context_history.is_empty(),
            "fresh entry should have no audit events"
        );
    }

    // ── Save-path isolation: no global orphan cleanup ───────────────────

    /// Regression for the Phase 2 fix: `save_blocking` previously ended every
    /// transaction with a global `DELETE FROM entries WHERE id NOT IN
    /// (SELECT entry_id FROM session_history)`. When `session_history` was
    /// transiently empty (as after migrate_v15's cascade), the next save wiped
    /// every entry in the database. This test proves saving one session can no
    /// longer delete an entry that belongs to no `session_history` row.
    #[test]
    fn save_blocking_does_not_delete_orphaned_entries_when_history_is_empty() {
        use crate::schema::entries;

        // Given a migrated database with FK=ON holding one orphan entry that no
        // `session_history` row references.
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("sessions.db");
        let database_url = db_path.to_string_lossy().to_string();
        let mut conn = SqliteConnection::establish(&database_url).expect("establish");
        sql_query("PRAGMA foreign_keys=ON")
            .execute(&mut conn)
            .expect("fk on");
        migrator::run_migrations(&mut conn).expect("migrations");
        sql_query(
            "INSERT INTO entries (id, timestamp, kind) \
             VALUES ('orphan-1', '2024-01-01T00:00:00Z', '\"User\"')",
        )
        .execute(&mut conn)
        .expect("seed orphan entry");

        // And an empty session_history (the post-cascade state).
        // When saving a fresh session with no entries.
        let fresh = ChatSessionState::new();
        save_blocking(&mut conn, &fresh).expect("save");

        // Then the orphan entry survives: `session_history` being empty for
        // this session did not trigger a global cleanup.
        let survivor: i64 = entries::table
            .filter(entries::id.eq("orphan-1"))
            .count()
            .get_result(&mut conn)
            .expect("count");
        assert_eq!(
            survivor, 1,
            "orphan entry must survive a save on another session"
        );
    }

    /// Standalone connection with migrations applied and FK on, for raw
    /// seeding and assertion in the orphan-scoping tests.
    fn migrated_conn() -> (tempfile::TempDir, SqliteConnection) {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("sessions.db");
        let url = db_path.to_string_lossy().to_string();
        let mut conn = SqliteConnection::establish(&url).expect("establish");
        sql_query("PRAGMA foreign_keys=ON")
            .execute(&mut conn)
            .expect("fk on");
        migrator::run_migrations(&mut conn).expect("migrations");
        (dir, conn)
    }

    #[tokio::test]
    async fn delete_reaps_entries_unique_to_deleted_session() {
        use crate::schema::entries;

        // Given a session with an entry that no other session references.
        let (_dir, mut conn) = migrated_conn();
        let session = make_session();
        let id = session.session_id().clone();
        save_blocking(&mut conn, &session).expect("save");

        // When deleting the session.
        delete_blocking(&mut conn, &id).expect("delete");

        // Then the unique entry is reaped (cleanup still works, not regressed
        // to never-reap).
        let surviving: i64 = entries::table.count().get_result(&mut conn).expect("count");
        assert_eq!(surviving, 0, "unique entry should be reaped on delete");
    }

    #[tokio::test]
    async fn delete_preserves_entries_shared_with_other_session() {
        use crate::schema::{entries, session_history};

        // Given a source session, forked so two sessions share the same entry
        // via their session_history junction rows.
        let (_dir, mut conn) = migrated_conn();
        let session = make_session();
        let source_id = session.session_id().clone();
        save_blocking(&mut conn, &session).expect("save source");
        let fork_id = fork_blocking(&mut conn, &source_id, 0).expect("fork");

        // The shared entry id is the one both sessions reference.
        let shared_entry: String = session_history::table
            .filter(session_history::session_id.eq(fork_id.to_string()))
            .select(session_history::entry_id)
            .first::<String>(&mut conn)
            .expect("load shared entry id");

        // When deleting the fork.
        delete_blocking(&mut conn, &fork_id).expect("delete fork");

        // Then the shared entry survives because the source session still
        // references it (the scoping filter held it back).
        let survivor: i64 = entries::table
            .filter(entries::id.eq(&shared_entry))
            .count()
            .get_result(&mut conn)
            .expect("count");
        assert_eq!(
            survivor, 1,
            "shared entry must survive deletion of one referencing session"
        );
    }

    #[tokio::test]
    async fn delete_does_not_reap_unrelated_orphan_entries() {
        use crate::schema::entries;

        // Given a store with one normal session and a pre-existing orphan entry
        // (no session_history row references it) belonging to no session.
        let (_dir, mut conn) = migrated_conn();
        let session = make_session();
        let id = session.session_id().clone();
        save_blocking(&mut conn, &session).expect("save");
        sql_query(
            "INSERT INTO entries (id, timestamp, kind) \
             VALUES ('orphan-x', '2024-01-01T00:00:00Z', '\"User\"')",
        )
        .execute(&mut conn)
        .expect("seed orphan");

        // When deleting the unrelated session.
        delete_blocking(&mut conn, &id).expect("delete");

        // Then the unrelated orphan entry survives — a delete of session A
        // never reaps entries that were never A's.
        let survivor: i64 = entries::table
            .filter(entries::id.eq("orphan-x"))
            .count()
            .get_result(&mut conn)
            .expect("count");
        assert_eq!(
            survivor, 1,
            "unrelated orphan entry must survive deletion of a different session"
        );
    }
}
