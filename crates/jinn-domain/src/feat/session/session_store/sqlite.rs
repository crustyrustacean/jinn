//! SQLite-backed session store implementation.
//!
//! Stores session data in normalized tables with a junction table for entries.
//! This eliminates duplication - each chat entry is stored once and shared
//! across sessions. The junction table enables fork support by copying only
//! small junction rows, not entry data.
//!
//! Backed by the `dao` crate: an async `Pool`/`Transaction` over `rusqlite`.
//! Single statements use `pool.execute`/`pool.query_*`; multi-statement
//! transactional bodies (save, delete, fork) use `pool.with_conn` to drive a
//! native rusqlite `transaction(|tx| …)` on one held connection.

use std::collections::HashMap;
use std::path::Path;

use async_trait::async_trait;
use daow::{Entity, FromRow, Pool, Row, dao};
use error_stack::{Report, ResultExt as _};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

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
#[derive(Debug, Clone, Copy)]
pub struct PoolConfig {
    /// Maximum number of connections in the pool.
    max_size: usize,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self { max_size: 4 }
    }
}

impl PoolConfig {
    /// Creates a new configuration with the given max pool size.
    #[must_use]
    pub const fn with_max_size(max_size: usize) -> Self {
        Self { max_size }
    }

    /// Returns the configured max pool size.
    #[must_use]
    pub const fn max_size(&self) -> usize {
        self.max_size
    }
}

/// SQLite-backed implementation of [`SessionStore`].
///
/// Holds a `dao` connection pool (`foreign_keys=ON`, `journal_mode=WAL`,
/// `busy_timeout=5000` applied automatically by the pool builder). Migrations
/// run on the pool before any store method is used.
pub struct SqliteSessionStore {
    pool: Pool,
}

impl SqliteSessionStore {
    /// Creates a new store using the platform-default sessions directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the sessions directory cannot be determined, the
    /// pool cannot be built, or migrations fail.
    pub async fn new() -> Result<Self, Report<SessionStoreError>> {
        // `sessions_dir()` already resolves to the canonical DB parent
        // (`~/.local/share/jinn` on Linux). An earlier revision appended an
        // extra `sessions` segment here, which silently split sessions across
        // two databases (`.../jinn/sessions.db` vs `.../jinn/sessions/sessions.db`).
        let dir = crate::common::app_paths::AppPaths::default().sessions_dir();
        Self::new_with_config(&dir, PoolConfig::default()).await
    }

    /// Creates a new store in the given directory, creating it if needed.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created, the pool cannot be
    /// built, or migrations fail.
    pub async fn new_in(dir: &Path) -> Result<Self, Report<SessionStoreError>> {
        Self::new_with_config(dir, PoolConfig::default()).await
    }

    /// Creates a new store in the given directory with a specific pool size.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created, the pool cannot be
    /// built, or migrations fail.
    pub async fn new_with_config(
        dir: &Path,
        config: PoolConfig,
    ) -> Result<Self, Report<SessionStoreError>> {
        std::fs::create_dir_all(dir)
            .change_context(SessionStoreError)
            .attach("failed to create sessions directory")?;
        let db_path = dir.join("sessions.db");
        Self::connect_at(&db_path, config).await
    }

    /// Opens or creates a store at an explicit database file path, creating
    /// any missing parent directories.
    ///
    /// # Errors
    ///
    /// Returns an error if the parent directories cannot be created, the pool
    /// cannot be built, or migrations fail.
    pub async fn open_or_create(file_path: &Path) -> Result<Self, Report<SessionStoreError>> {
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)
                .change_context(SessionStoreError)
                .attach("failed to create parent directory for database file")?;
        }
        Self::connect_at(file_path, PoolConfig::default()).await
    }

    /// Builds the pool at `db_path`, then runs migrations on it.
    ///
    /// The `dao` `Pool::builder` applies `foreign_keys=ON`, `journal_mode=WAL`,
    /// and `busy_timeout=5000` to every freshly-opened connection, so the
    /// per-connection pragma customizer is no longer needed.
    async fn connect_at(
        db_path: &Path,
        config: PoolConfig,
    ) -> Result<Self, Report<SessionStoreError>> {
        let url = db_path.to_string_lossy().to_string();
        let pool = {
            let mut builder = Pool::builder().path(url).max_size(config.max_size);
            // The sessions store runs pragmas via the pool. Override journal_mode
            // to WAL explicitly so it is recorded even if dao's defaults change.
            builder = builder.pragma("journal_mode", "WAL");
            builder = builder.pragma("foreign_keys", "ON");
            builder = builder.pragma("busy_timeout", "5000");
            builder.build()
        }
        .change_context(SessionStoreError)
        .attach("failed to create connection pool")?;

        migrator::run_migrations(&pool)
            .await
            .change_context(SessionStoreError)
            .attach("failed to run database migrations")?;

        Ok(Self { pool })
    }
}

impl std::fmt::Debug for SqliteSessionStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteSessionStore")
            .field("backend", &"daow::Pool<sqlite>")
            .finish()
    }
}

#[async_trait]
impl SessionStore for SqliteSessionStore {
    fn name(&self) -> &'static str {
        "sqlite"
    }

    async fn save(&self, session: &ChatSessionState) -> Result<(), Report<SessionStoreError>> {
        // Non-persistent sessions (e.g. plugin one-shots) never touch the store.
        if !session.core.persist {
            return Ok(());
        }
        let row = NewSessionRow::try_from(session)?;
        save_in_transaction(&self.pool, session, &row).await
    }

    async fn load_summaries(&self) -> Result<Vec<SessionSummary>, Report<SessionStoreError>> {
        let dao = SessionDao::new(self.pool.clone());
        let rows: Vec<SessionRow> = dao
            .all_sessions()
            .await
            .change_context(SessionStoreError)
            .attach("failed to query summaries")?;
        Ok(rows.into_iter().map(summary_from_row).collect())
    }

    async fn load_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<ChatSessionState>, Report<SessionStoreError>> {
        let session_id_str = session_id.to_string();

        // Load session metadata.
        let dao = SessionDao::new(self.pool.clone());
        let meta: Option<SessionRow> = dao
            .session_by_id(session_id_str.clone())
            .await
            .change_context(SessionStoreError)
            .attach("failed to query session metadata")?;

        let Some(meta) = meta else {
            return Ok(None);
        };

        // Load entries via junction table, ordered by ordinal.
        let joined: Vec<JoinedEntry> = self
            .pool
            .query_all(
                "SELECT entries.id AS entry_id, entries.timing AS timing, entries.kind AS kind, \
                 entries.context_history AS context_history, \
                 session_history.pin_position AS pin_position, \
                 session_history.ignored AS ignored, \
                 session_history.context_override AS context_override \
                 FROM entries \
                 INNER JOIN session_history ON entries.id = session_history.entry_id \
                 WHERE session_history.session_id = ? \
                 ORDER BY session_history.ordinal ASC",
                vec![Box::new(session_id_str.clone())],
            )
            .await
            .change_context(SessionStoreError)
            .attach("failed to query entries")?;

        let entries: Vec<ChatEntry> = joined.into_iter().map(entry_from_joined).collect();

        // Load token ledger.
        let ledger_rows: Vec<TokenLedgerRow> = self
            .pool
            .query_all(
                "SELECT id, session_id, timestamp, tokens_sent, tokens_received, cost, model_used \
                 FROM token_ledger WHERE session_id = ?",
                vec![Box::new(session_id_str.clone())],
            )
            .await
            .change_context(SessionStoreError)
            .attach("failed to query token ledger")?;

        let ledger: Vec<TokenRecord> = ledger_rows.into_iter().map(record_from_row).collect();

        // Reconstruct ChatSessionState via exhaustive destructuring.
        let session = ChatSessionState::try_from(SessionLoadContext {
            row: meta,
            entries,
            ledger,
        })?;

        Ok(Some(session))
    }

    async fn delete(&self, session_id: &SessionId) -> Result<(), Report<SessionStoreError>> {
        let session_id_str = session_id.to_string();
        self.pool
            .with_conn(move |conn| delete_with_scoped_reaping(conn, &session_id_str))
            .await
            .change_context(SessionStoreError)
            .attach("failed to delete session")?;
        Ok(())
    }

    async fn fork(
        &self,
        source_session_id: &SessionId,
        at_ordinal: usize,
    ) -> Result<SessionId, Report<SessionStoreError>> {
        let source_str = source_session_id.to_string();
        let new_id = SessionId::new();
        let new_id_str = new_id.to_string();

        self.pool
            .with_conn(move |conn| fork_in_transaction(conn, &source_str, &new_id_str, at_ordinal))
            .await
            .change_context(SessionStoreError)
            .attach("failed to fork session")?;

        Ok(new_id)
    }

    async fn set_archived(
        &self,
        session_id: &SessionId,
        archived: bool,
    ) -> Result<(), Report<SessionStoreError>> {
        let session_id_str = session_id.to_string();
        let dao = SessionDao::new(self.pool.clone());
        dao.set_archived(archived, session_id_str)
            .await
            .change_context(SessionStoreError)
            .attach("failed to set archived flag")?;
        Ok(())
    }

    async fn load_unarchived_summaries(
        &self,
    ) -> Result<Vec<SessionSummary>, Report<SessionStoreError>> {
        let dao = SessionDao::new(self.pool.clone());
        let rows: Vec<SessionRow> = dao
            .unarchived_sessions()
            .await
            .change_context(SessionStoreError)
            .attach("failed to query unarchived summaries")?;
        Ok(rows.into_iter().map(summary_from_row).collect())
    }

    async fn shutdown(&self) -> Result<(), Report<SessionStoreError>> {
        let result: Option<CheckpointResult> = self
            .pool
            .query_one("PRAGMA wal_checkpoint(TRUNCATE)", vec![])
            .await
            .change_context(SessionStoreError)
            .attach("failed to run wal_checkpoint(TRUNCATE) during shutdown")?;
        if let Some(result) = result {
            classify_checkpoint_result(&result);
        }
        Ok(())
    }
}

// ── Row models ───────────────────────────────────────────────────────────

/// Reading model for the `sessions` table (post-v20: 9 authoritative columns).
///
/// All columns are now authoritative — the six "zombie" columns
/// (`profile`, `blobs`, `cwd`, `lifecycle_name`, `lifecycle_args`,
/// `lifecycle_script_state`) were dropped by migration v20 after the metadata
/// blob was backfilled for every row. The metadata JSON blob is the single
/// source of truth for session core fields.
#[derive(Debug, Clone, Entity)]
#[dao(table = "sessions")]
struct SessionRow {
    #[dao(pk)]
    id: String,
    title: Option<String>,
    updated_at: String,
    created_at: String,
    parent_session: Option<String>,
    archived: bool,
    metadata: Option<String>,
    is_automated: bool,
    persist: bool,
}

/// Insert model for the `sessions` table. Built from a `ChatSessionState` then
/// upserted via hand-written SQL (full-column upsert is behavior-preserving:
/// immutable columns like `created_at` are re-written with their unchanged
/// values).
struct NewSessionRow {
    id: String,
    title: Option<String>,
    updated_at: String,
    created_at: String,
    parent_session: Option<String>,
    archived: bool,
    metadata: Option<String>,
    is_automated: bool,
}

/// A joined `entries` + `session_history` row for loading a session's entries.
///
/// Read by a manual `FromRow` that maps the aliased columns of the JOIN query.
struct JoinedEntry {
    entry_id: String,
    timing: String,
    kind: String,
    context_history: String,
    pin_position: Option<String>,
    ignored: bool,
    context_override: String,
}

impl FromRow for JoinedEntry {
    fn from_row(row: &Row) -> daow::Result<Self> {
        Ok(Self {
            entry_id: row.get("entry_id")?,
            timing: row.get("timing")?,
            kind: row.get("kind")?,
            context_history: row.get("context_history")?,
            pin_position: row.get("pin_position")?,
            ignored: row.get("ignored")?,
            context_override: row.get("context_override")?,
        })
    }
}

/// Reading model for the `token_ledger` table.
#[derive(Debug, Clone, Entity)]
#[dao(table = "token_ledger")]
struct TokenLedgerRow {
    #[dao(pk)]
    id: i64,
    session_id: String,
    timestamp: String,
    tokens_sent: i32,
    tokens_received: i32,
    cost: Option<f64>,
    model_used: Option<String>,
}

// ── Typed DAO traits (compile-time SQL validation via DAOW_DATABASE_URL) ───

/// Session-level queries that run directly on the pool. These use `#[query]` /
/// `#[execute]` so the `dao` macro validates the SQL against the post-v20 schema
/// at compile time (see `jinn-session-schema` + `build.rs`). Transactional multi-statement
/// bodies (`save`, `delete`, `fork`) still use `pool.with_conn` with raw rusqlite
/// because they need dynamic `IN (?, ?, …)` placeholder strings that cannot be
/// statically validated.
#[dao]
#[async_trait]
trait SessionDao {
    #[query(
        "SELECT id, title, updated_at, created_at, parent_session, archived, metadata, is_automated, persist FROM sessions"
    )]
    async fn all_sessions(&self) -> daow::Result<Vec<SessionRow>>;

    #[query(
        "SELECT id, title, updated_at, created_at, parent_session, archived, metadata, is_automated, persist FROM sessions WHERE id = ?"
    )]
    async fn session_by_id(&self, id: String) -> daow::Result<Option<SessionRow>>;

    #[query(
        "SELECT id, title, updated_at, created_at, parent_session, archived, metadata, is_automated, persist FROM sessions WHERE archived = FALSE"
    )]
    async fn unarchived_sessions(&self) -> daow::Result<Vec<SessionRow>>;

    #[execute("UPDATE sessions SET archived = ? WHERE id = ?")]
    async fn set_archived(&self, archived: bool, id: String) -> daow::Result<daow::ExecuteResult>;
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
pub(crate) struct PersistableCore {
    session_id: SessionId,
    title: Option<String>,
    updated_at: jiff::Timestamp,
    created_at: jiff::Timestamp,
    profile: SessionProfile,
    cwd: std::path::PathBuf,
    parent_session: Option<SessionId>,
    /// Highest entry ordinal inherited from parent at fork time.
    /// `None` for root sessions.
    #[serde(default)]
    fork_ordinal: Option<usize>,

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
    attached_plugins: Vec<jinn_core_types::AttachedPlugin>,
    /// Whether this session should be persisted to disk.
    /// Defaults to true for blobs written by older versions.
    #[serde(default = "crate::feat::session::chat_session::default_persist")]
    persist: bool,
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
            fork_ordinal: core.fork_ordinal,
            blobs: core.blobs.clone(),
            lifecycle_name: core.lifecycle_name.clone(),
            lifecycle_args: core.lifecycle_args.clone(),
            lifecycle_script_state: core.lifecycle_script_state,
            task_list: core.task_list.clone(),
            attached_plugins: core.attached_plugins.clone(),
            persist: core.persist,
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
            fork_ordinal: core.fork_ordinal,
            blobs: core.blobs,
            lifecycle_name: core.lifecycle_name,
            lifecycle_args: core.lifecycle_args,
            session_state: SessionState::Loaded, // overridden by TryFrom<SessionLoadContext> from archived column
            lifecycle_script_state: core.lifecycle_script_state,
            ephemeral: SessionCoreEphemeral::default(),
            is_automated: false,      // set from DB column after deserialization
            assembly_overrides: None, // runtime-only, never persisted
            has_interacted: false, // restored sessions get mark_interacted() in handle_session_load_completed
            task_list: core.task_list,
            attached_plugins: core.attached_plugins,
            persist: core.persist,
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
                    profile: _profile, // persisted via metadata blob
                    cwd: _cwd,         // persisted via metadata blob
                    token_ledger: _ledger, // persisted via token_ledger table below
                    parent_session,

                    fork_ordinal: _fork_ordinal, // included in metadata blob via PersistableCore

                    blobs: _blobs,                   // persisted via metadata blob
                    lifecycle_name: _lifecycle_name, // persisted via metadata blob
                    lifecycle_args: _lifecycle_args, // persisted via metadata blob
                    ephemeral: _ephemeral,           // runtime-only state, not persisted
                    session_state,
                    lifecycle_script_state: _lifecycle_script_state, // persisted via metadata blob
                    is_automated,
                    persist: _persist, // persisted via metadata blob
                    assembly_overrides: _assembly_overrides, // runtime-only, not persisted
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
            parent_session: parent_session
                .as_ref()
                .map(std::string::ToString::to_string),
            archived: *session_state == SessionState::Archived,
            metadata: Some(
                serde_json::to_string(&PersistableCore::from(&session.core))
                    .change_context(SessionStoreError)
                    .attach("failed to serialize metadata")?,
            ),
            is_automated: *is_automated,
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
            id: _,
            title: _,
            updated_at: _,
            created_at: _,
            parent_session: _,
            archived,
            metadata,
            is_automated,
            persist: _persist, // column value used by PersistableCore round-trip
        } = ctx.row;

        // Post-v20 every row has a metadata blob (v20 backfilled any NULL rows
        // from the dropped zombie columns). Deserialize it as the authoritative
        // source of truth for SessionCore fields, then overlay the
        // normalized-table data (entries, token_ledger).
        let metadata_json = metadata.ok_or_else(|| {
            Report::new(SessionStoreError)
                .attach("session row has NULL metadata after v20 (data corruption)")
        })?;
        let persistable: PersistableCore = serde_json::from_str(&metadata_json)
            .change_context(SessionStoreError)
            .attach("failed to deserialize session metadata blob")?;
        let mut core = SessionCore::from(persistable);

        // Single source of truth: is_automated column → core.is_automated
        core.is_automated = is_automated;

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

// ── Transactions ─────────────────────────────────────────────────────────

/// Saves a complete session in a single transaction.
///
/// Upserts session metadata, replaces all junction rows and token ledger rows,
/// and inserts any new entries. Orphaned-entry reaping is intentionally not done
/// here — it belongs in `delete`/`fork`, where the removing session is known. A
/// global cleanup in the save hot-path could wipe every entry if
/// `session_history` is transiently empty (e.g. mid-migration).
fn save_in_transaction<'a>(
    pool: &'a Pool,
    session: &'a ChatSessionState,
    row: &'a NewSessionRow,
) -> impl std::future::Future<Output = Result<(), Report<SessionStoreError>>> + Send + 'a {
    // Clone the per-entry data up front so the closure is `'static`-able across
    // the spawn_blocking boundary. The history + ledger are needed inside the tx.
    let entries = persistable_entries(session);
    let ledger = persistable_ledger(session);
    let row_id = row.id.clone();
    let row_title = row.title.clone();
    let row_updated_at = row.updated_at.clone();
    let row_created_at = row.created_at.clone();
    let row_parent = row.parent_session.clone();
    let row_archived = row.archived;
    let row_metadata = row.metadata.clone();
    let row_is_automated = row.is_automated;

    async move {
        pool.with_conn(move |conn| -> daow::Result<()> {
            // rusqlite 0.40: `transaction()` returns a `Transaction<'_>` that
            // derefs to `Connection` and must be committed explicitly.
            let tx = conn.transaction()?;
            upsert_session_row(
                &tx,
                &row_id,
                &row_title,
                &row_updated_at,
                &row_created_at,
                &row_parent,
                row_archived,
                &row_metadata,
                row_is_automated,
            )?;

            // Delete existing junction rows and token ledger for this session.
            tx.execute(
                "DELETE FROM session_history WHERE session_id = ?",
                rusqlite::params![&row_id],
            )?;
            tx.execute(
                "DELETE FROM token_ledger WHERE session_id = ?",
                rusqlite::params![&row_id],
            )?;

            for entry in &entries {
                insert_entry_and_junction(&tx, &row_id, entry)?;
            }
            for record in &ledger {
                insert_token_ledger_row(&tx, &row_id, record)?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
        .change_context(SessionStoreError)
        .attach("failed to save session")?;
        Ok(())
    }
}

/// Builds the list of persistable entries (skipping transient UI hints).
fn persistable_entries(session: &ChatSessionState) -> Vec<PersistableEntry> {
    session
        .history()
        .iter()
        .enumerate()
        .filter(|(_, e)| !matches!(e.kind, ChatEntryKind::Transient(_)))
        .map(|(ordinal, entry)| PersistableEntry::build(entry, ordinal))
        .collect()
}

/// Builds the list of persistable token ledger records.
fn persistable_ledger(session: &ChatSessionState) -> Vec<PersistableTokenRecord> {
    session
        .token_ledger()
        .iter()
        .map(PersistableTokenRecord::build)
        .collect()
}

/// A pre-serialized entry ready for INSERT (all SQL params computed once).
struct PersistableEntry {
    entry_id: String,
    timing: String,
    kind: String,
    context_history: String,
    ordinal: i32,
    pin_position: Option<String>,
    ignored: bool,
    context_override: String,
}

impl PersistableEntry {
    /// Serializes an entry's fields into the SQL-ready form.
    fn build(entry: &ChatEntry, ordinal: usize) -> Self {
        let timing = serde_json::to_string(&entry.timing).unwrap_or_else(|_| "{}".to_owned());
        let kind = serde_json::to_string(&entry.kind).unwrap_or_else(|_| "{}".to_owned());
        let context_history =
            serde_json::to_string(&entry.context_history).unwrap_or_else(|_| "[]".to_owned());
        let pin_position = entry.pin_position.map(|p| p.to_string());
        let context_override = serde_json::to_string(&entry.context_override())
            .unwrap_or_else(|_| "\"default\"".to_owned());
        Self {
            entry_id: entry.id.to_string(),
            timing,
            kind,
            context_history,
            ordinal: ordinal as i32,
            pin_position,
            ignored: entry.ignored(),
            context_override,
        }
    }
}

/// A pre-serialized token ledger row.
struct PersistableTokenRecord {
    timestamp: String,
    tokens_sent: i32,
    tokens_received: i32,
    cost: Option<f64>,
    model_used: Option<String>,
}

impl PersistableTokenRecord {
    /// Serializes a `TokenRecord` into the SQL-ready form.
    fn build(record: &TokenRecord) -> Self {
        Self {
            timestamp: record.timestamp.to_string(),
            tokens_sent: record.tokens_sent as i32,
            tokens_received: record.tokens_received as i32,
            cost: record.cost,
            model_used: record.model_used.clone(),
        }
    }
}

/// Upserts a session row (full-column; immutable columns are no-ops on re-write).
fn upsert_session_row(
    conn: &rusqlite::Connection,
    id: &str,
    title: &Option<String>,
    updated_at: &str,
    created_at: &str,
    parent_session: &Option<String>,
    archived: bool,
    metadata: &Option<String>,
    is_automated: bool,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO sessions (id, title, updated_at, created_at, parent_session, archived, \
         metadata, is_automated, persist) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, TRUE) \
         ON CONFLICT(id) DO UPDATE SET \
         title = excluded.title, \
         updated_at = excluded.updated_at, \
         created_at = excluded.created_at, \
         parent_session = excluded.parent_session, \
         archived = excluded.archived, \
         metadata = excluded.metadata, \
         is_automated = excluded.is_automated, \
         persist = excluded.persist",
        rusqlite::params![
            id,
            title,
            updated_at,
            created_at,
            parent_session,
            archived,
            metadata,
            is_automated
        ],
    )?;
    Ok(())
}

/// Inserts an entry row (upserting `context_history`) and its junction row.
fn insert_entry_and_junction(
    conn: &rusqlite::Connection,
    session_id: &str,
    entry: &PersistableEntry,
) -> rusqlite::Result<()> {
    // Insert entry. On conflict (entry shared across sessions), update
    // context_history since it mutates after first insertion via
    // `apply_context_override`.
    conn.execute(
        "INSERT INTO entries (id, timing, kind, context_history) \
         VALUES (?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET context_history = excluded.context_history",
        rusqlite::params![
            entry.entry_id,
            entry.timing,
            entry.kind,
            entry.context_history
        ],
    )?;

    // Insert junction row.
    conn.execute(
        "INSERT INTO session_history \
         (session_id, entry_id, ordinal, pin_position, ignored, context_override) \
         VALUES (?, ?, ?, ?, ?, ?)",
        rusqlite::params![
            session_id,
            entry.entry_id,
            entry.ordinal,
            entry.pin_position,
            entry.ignored,
            entry.context_override,
        ],
    )?;
    Ok(())
}

/// Inserts a token ledger row.
fn insert_token_ledger_row(
    conn: &rusqlite::Connection,
    session_id: &str,
    record: &PersistableTokenRecord,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO token_ledger \
         (session_id, timestamp, tokens_sent, tokens_received, cost, model_used) \
         VALUES (?, ?, ?, ?, ?, ?)",
        rusqlite::params![
            session_id,
            record.timestamp,
            record.tokens_sent,
            record.tokens_received,
            record.cost,
            record.model_used,
        ],
    )?;
    Ok(())
}

// ── Scoped orphan reaping (delete) ───────────────────────────────────────

/// Deletes a session and reaps entries that became orphaned by this delete.
///
/// Cleanup is **scoped to this session's own former entries**: the session's
/// `entry_id`s are captured before the FK cascade removes its junction rows,
/// then only those candidates that no remaining session references are deleted.
/// A global orphan sweep is deliberately avoided — a transiently-empty global
/// `session_history` state can never cause mass reaping of other sessions' data.
fn delete_with_scoped_reaping(
    conn: &mut rusqlite::Connection,
    session_id_str: &str,
) -> daow::Result<()> {
    let tx = conn.transaction()?;
    // Capture this session's entry references before the FK cascade
    // removes them. These are the only candidates for reaping.
    let candidates: Vec<String> = tx
        .prepare("SELECT DISTINCT entry_id FROM session_history WHERE session_id = ?")?
        .query_map([session_id_str], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    // Delete the session. With FK=ON this cascades to remove this session's
    // session_history and token_ledger rows.
    tx.execute(
        "DELETE FROM sessions WHERE id = ?",
        rusqlite::params![session_id_str],
    )?;

    if !candidates.is_empty() {
        // After the cascade, session_history holds only OTHER sessions'
        // references. Reap a candidate only if no remaining session claims it.
        let orphaned = unreferenced_entries(&tx, &candidates)?;
        if !orphaned.is_empty() {
            delete_entries(&tx, &orphaned)?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Returns the subset of `candidates` that no remaining `session_history` row references.
fn unreferenced_entries(
    conn: &rusqlite::Connection,
    candidates: &[String],
) -> rusqlite::Result<Vec<String>> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = repeat_placeholders(candidates.len());
    let sql = format!("SELECT entry_id FROM session_history WHERE entry_id IN ({placeholders})");
    let referenced: Vec<String> = {
        let mut stmt = conn.prepare(&sql)?;
        let params = candidates
            .iter()
            .map(|c| c as &dyn rusqlite::ToSql)
            .collect::<Vec<_>>();
        stmt.query_map(params.as_slice(), |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let orphaned = candidates
        .iter()
        .filter(|id| !referenced.iter().any(|r| r == *id))
        .cloned()
        .collect();
    Ok(orphaned)
}

/// Deletes the given entry rows by id.
fn delete_entries(conn: &rusqlite::Connection, ids: &[String]) -> rusqlite::Result<()> {
    let placeholders = repeat_placeholders(ids.len());
    let sql = format!("DELETE FROM entries WHERE id IN ({placeholders})");
    let params = ids
        .iter()
        .map(|id| id as &dyn rusqlite::ToSql)
        .collect::<Vec<_>>();
    conn.execute(&sql, params.as_slice())?;
    Ok(())
}

/// Builds a `?, ?, …` placeholder string of `n` elements.
fn repeat_placeholders(n: usize) -> String {
    std::iter::repeat("?")
        .take(n)
        .collect::<Vec<_>>()
        .join(", ")
}

// ── Fork ─────────────────────────────────────────────────────────────────

/// Forks a session from a specific entry ordinal.
///
/// Creates a new session with `parent_session` = source, copies junction rows
/// up to and including `at_ordinal`. Entry data is shared (not duplicated).
fn fork_in_transaction(
    conn: &mut rusqlite::Connection,
    source_str: &str,
    new_id_str: &str,
    at_ordinal: usize,
) -> daow::Result<()> {
    let tx = conn.transaction()?;
    // Load source session metadata.
    let source_meta: Option<(Option<String>, Option<String>, bool)> = tx
        .query_row(
            "SELECT title, metadata, is_automated FROM sessions WHERE id = ?",
            rusqlite::params![source_str],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            },
        )
        .ok();

    let Some((title, metadata, is_automated)) = source_meta else {
        return Err(daow::Error::Custom(
            "source session not found for fork".to_string(),
        ));
    };

    let now = jiff::Timestamp::now().to_string();
    let forked_metadata = fork_metadata(metadata.as_ref(), source_str, new_id_str, at_ordinal);

    // Create new session row.
    tx.execute(
        "INSERT INTO sessions (id, title, updated_at, created_at, parent_session, archived, \
         metadata, is_automated, persist) \
         VALUES (?, ?, ?, ?, ?, FALSE, ?, ?, TRUE)",
        rusqlite::params![
            new_id_str,
            title,
            now.clone(),
            now, // fresh created_at - it's a new session
            source_str,
            forked_metadata,
            is_automated,
        ],
    )?;

    // Copy junction rows up to and including at_ordinal.
    tx.execute(
        "INSERT INTO session_history \
         (session_id, entry_id, ordinal, pin_position, ignored, context_override) \
         SELECT ?, entry_id, ordinal, pin_position, ignored, context_override \
         FROM session_history \
         WHERE session_id = ? AND ordinal <= ?",
        rusqlite::params![new_id_str, source_str, at_ordinal as i32],
    )?;
    tx.commit()?;
    Ok(())
}

/// Patches a metadata JSON blob for a forked session.
///
/// Overrides `parent_session`, `session_id`, `created_at`, and `updated_at`
/// so the forked session's metadata reflects its new identity.
/// Falls back to `None` if deserialization or re-serialization fails.
fn fork_metadata(
    source_metadata: Option<&String>,
    source_id_str: &str,
    new_id_str: &str,
    at_ordinal: usize,
) -> Option<String> {
    let json = source_metadata.as_ref()?;
    let mut core: PersistableCore = serde_json::from_str(json).ok()?;
    core.parent_session = Some(SessionId::from(source_id_str.to_owned()));
    core.session_id = SessionId::from(new_id_str.to_owned());
    core.created_at = jiff::Timestamp::now();
    core.updated_at = jiff::Timestamp::now();
    core.fork_ordinal = Some(at_ordinal);
    serde_json::to_string(&core).ok()
}

// ── Row → domain conversions ─────────────────────────────────────────────

/// Builds a `SessionSummary` from a loaded `SessionRow`.
fn summary_from_row(row: SessionRow) -> SessionSummary {
    SessionSummary {
        session_id: SessionId::from(row.id),
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
    }
}

/// Reconstructs a `ChatEntry` from a joined entry/junction row.
fn entry_from_joined(joined: JoinedEntry) -> ChatEntry {
    let kind: ChatEntryKind = serde_json::from_str(&joined.kind).unwrap_or_else(|e| {
        tracing::warn!(entry_id = %joined.entry_id, error = %e, "failed to deserialize entry kind");
        ChatEntryKind::Error(format!("corrupt entry: {e}"))
    });
    let pin_position = joined.pin_position.as_deref().and_then(|s| match s {
        "TOP" => Some(crate::protocol::PinPosition::Top),
        "BOTTOM" => Some(crate::protocol::PinPosition::Bottom),
        "RELATIVE" => Some(crate::protocol::PinPosition::Relative),
        _ => None,
    });

    let timing: crate::protocol::EntryTiming =
        serde_json::from_str(&joined.timing).unwrap_or_else(|_| {
            // Fallback: parse raw timestamp string as Instant (legacy data).
            joined.timing.parse::<jiff::Timestamp>().map_or_else(
                |_| crate::protocol::EntryTiming::instant_now(),
                |at| crate::protocol::EntryTiming::Instant { at },
            )
        });
    let mut chat_entry = ChatEntry::new_with_kind(
        ChatEntryId::from(joined.entry_id),
        timing,
        kind,
        pin_position,
    );
    // Restored from DB - no audit event recorded.
    let override_value: ContextOverride = serde_json::from_str(&joined.context_override)
        .unwrap_or_else(|e| {
            tracing::warn!(
                entry_id = %chat_entry.id.as_uuid(),
                raw = %joined.context_override,
                error = %e,
                "failed to deserialize context_override, falling back to Default"
            );
            // Fallback: use legacy ignored column if context_override is corrupt
            if joined.ignored {
                ContextOverride::ForcedExclude
            } else {
                ContextOverride::Default
            }
        });
    chat_entry.restore_context_override(override_value);

    // Restore audit trail. Empty array (default) loads as Vec::new().
    // Corrupt JSON falls back to empty with a warning.
    chat_entry.context_history =
        serde_json::from_str(&joined.context_history).unwrap_or_else(|e| {
            tracing::warn!(
                entry_id = %chat_entry.id.as_uuid(),
                raw = %joined.context_history,
                error = %e,
                "failed to deserialize context_history, falling back to empty"
            );
            Vec::new()
        });
    chat_entry
}

/// Reconstructs a `TokenRecord` from a `TokenLedgerRow`.
fn record_from_row(row: TokenLedgerRow) -> TokenRecord {
    TokenRecord {
        model_used: row.model_used,
        timestamp: row
            .timestamp
            .parse()
            .unwrap_or_else(|_| jiff::Timestamp::now()),
        tokens_sent: row.tokens_sent as u32,
        tokens_received: row.tokens_received as u32,
        cost: row.cost,
    }
}

// ── Shutdown checkpoint ────────────────────���─────────────────────────────

/// Result row of `PRAGMA wal_checkpoint(TRUNCATE)`.
///
/// Columns: `busy` (1 if the checkpoint could not complete because a reader
/// held a snapshot), `log` (frames in the WAL), `checkpointed` (frames folded
/// into the main db). Read by name via a manual `FromRow`.
struct CheckpointResult {
    busy: i64,
    log: i64,
    checkpointed: i64,
}

impl FromRow for CheckpointResult {
    fn from_row(row: &Row) -> daow::Result<Self> {
        Ok(Self {
            busy: row.get("busy")?,
            log: row.get("log")?,
            checkpointed: row.get("checkpointed")?,
        })
    }
}

/// Classifies a `wal_checkpoint` result row as fatal or non-fatal.
///
/// Extracted from `shutdown` as a pure function so the `busy=1`
/// graceful-degradation path is unit-testable without a database. A busy
/// result (a reader held a snapshot mid-checkpoint) logs a warning and returns
/// `Ok` — the un-folded frames survive in the WAL and fold on the next open.
/// A clean result logs at info level.
///
/// This function never fails: the only fallible step in shutdown is the
/// checkpoint query itself, which stays in `shutdown`. This pure classifier
/// just chooses the log level based on the result row.
fn classify_checkpoint_result(result: &CheckpointResult) {
    if result.busy == 1 {
        tracing::warn!(
            log_frames = result.log,
            checkpointed_frames = result.checkpointed,
            "wal_checkpoint was busy; some WAL frames remain un-folded (non-fatal)"
        );
    } else {
        tracing::info!(
            log_frames = result.log,
            checkpointed_frames = result.checkpointed,
            "folded WAL into sessions.db during shutdown"
        );
    }
}
