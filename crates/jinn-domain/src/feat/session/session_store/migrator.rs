//! Database migration runner.
//!
//! Runs schema migrations sequentially using a `_migrations` tracking table.
//! Each migration is a dedicated function. The runner checks the current
//! version and runs only pending migrations in order.
//!
//! # Version tracking
//!
//! The `_migrations` table records each completed migration. On startup the
//! runner reads the highest version and skips any migrations already applied.
//! Each migration is recorded **after** it succeeds, so a failed migration
//! will be re-attempted on the next startup.

use diesel::prelude::*;
use diesel::sql_query;
use error_stack::{Report, ResultExt as _};

use super::SessionStoreError;

/// Runs all pending database migrations.
///
/// Migrations run with `PRAGMA foreign_keys=OFF`. This is essential: DDL such as
/// `DROP TABLE sessions` (used by table-rebuild migrations like `migrate_v15`)
/// performs an implicit `DELETE` of all rows, which would otherwise fire the
/// application-level `ON DELETE CASCADE` and wipe every `session_history` and
/// `token_ledger` row. After the migrations complete (or fail), FK is re-enabled
/// and `PRAGMA foreign_key_check` verifies referential integrity.
///
/// Bootstraps the `_migrations` tracking table, reads the current version,
/// and runs any migrations that haven't been applied yet.
///
/// # Errors
///
/// Returns an error if any migration fails, if the FK pragma cannot be
/// toggled, or if `foreign_key_check` reports integrity violations after
/// the migration run.
pub fn run_migrations(conn: &mut SqliteConnection) -> Result<(), Report<SessionStoreError>> {
    // Disable FK for the migration run. The pragma is per-connection and a
    // no-op inside an active transaction, so it must be set here (outside any
    // transaction) rather than inside `run_pending_migrations`.
    sql_query("PRAGMA foreign_keys=OFF")
        .execute(conn)
        .change_context(SessionStoreError)
        .attach("failed to disable foreign_keys for migration")?;

    let migrate_result = run_pending_migrations(conn);

    // Always re-enable FK and verify integrity, even if a migration failed,
    // so the connection is left in its normal (FK-on) state regardless of
    // outcome.
    sql_query("PRAGMA foreign_keys=ON")
        .execute(conn)
        .change_context(SessionStoreError)
        .attach("failed to re-enable foreign_keys after migration")?;

    // `foreign_key_check` returns one row per FK violation (empty = clean).
    let violations: Vec<FkCheckRow> = sql_query("PRAGMA foreign_key_check")
        .load(conn)
        .change_context(SessionStoreError)
        .attach("failed to run foreign_key_check after migration")?;
    if !violations.is_empty() {
        let tables: Vec<&str> = violations.iter().map(|v| v.table.as_str()).collect();
        return Err(Report::new(SessionStoreError)
            .attach("foreign_key_check reported violations after migration")
            .attach(format!("violating tables: {}", tables.join(", "))));
    }

    migrate_result
}

/// Runs all pending migrations in order.
///
/// Called by [`run_migrations`] with foreign keys disabled. Bootstraps the
/// tracking table, reads the current version, and runs every unapplied
/// migration sequentially.
///
/// # Errors
///
/// Returns an error if any migration or version recording fails.
fn run_pending_migrations(conn: &mut SqliteConnection) -> Result<(), Report<SessionStoreError>> {
    bootstrap_tracking_table(conn)?;
    let current = current_version(conn)?;

    if current < 0 {
        migrate_v0(conn)?;
        record_version(conn, 0, "create_initial_schema")?;
    }
    if current < 1 {
        migrate_v1(conn)?;
        record_version(conn, 1, "add_cwd_column")?;
    }
    if current < 2 {
        migrate_v2(conn)?;
        record_version(conn, 2, "add_created_at_column")?;
    }
    if current < 3 {
        migrate_v3(conn)?;
        record_version(conn, 3, "add_ignored_to_session_entries")?;
    }
    if current < 4 {
        migrate_v4(conn)?;
        record_version(conn, 4, "add_cost_to_token_ledger")?;
    }
    if current < 5 {
        migrate_v5(conn)?;
        record_version(conn, 5, "add_lifecycle_columns_to_sessions")?;
    }
    if current < 6 {
        migrate_v6(conn);
        record_version(conn, 6, "add_archived_column")?;
    }
    if current < 7 {
        migrate_v7(conn)?;
        record_version(conn, 7, "add_lifecycle_script_state_column")?;
    }
    if current < 8 {
        migrate_v8(conn)?;
        record_version(conn, 8, "add_metadata_column")?;
    }
    if current < 9 {
        migrate_v9(conn)?;
        record_version(conn, 9, "rename_session_entries_to_session_history")?;
    }
    if current < 10 {
        migrate_v10(conn)?;
        record_version(conn, 10, "consolidate_to_compaction_strategy")?;
    }
    if current < 11 {
        migrate_v11(conn)?;
        record_version(conn, 11, "add_is_workflow_column")?;
    }
    if current < 12 {
        migrate_v12(conn)?;
        record_version(conn, 12, "replace_ignored_with_context_override")?;
    }
    if current < 13 {
        migrate_v13(conn)?;
        record_version(conn, 13, "add_judge_meta_column")?;
    }
    if current < 14 {
        migrate_v14(conn)?;
        record_version(conn, 14, "add_context_history")?;
    }
    if current < 15 {
        migrate_v15(conn)?;
        record_version(conn, 15, "drop_strategy_state_column")?;
    }
    if current < 16 {
        migrate_v16(conn)?;
        record_version(
            conn,
            16,
            "rename_is_workflow_to_is_automated_and_add_persist",
        )?;
    }
    if current < 17 {
        migrate_v17(conn)?;
        record_version(
            conn,
            17,
            "rewrite_model_to_model_selection_and_add_model_used",
        )?;
    }
    if current < 18 {
        migrate_v18(conn)?;
        record_version(conn, 18, "rename_entries_timestamp_to_timing")?;
    }
    Ok(())
}

// ── Tracking table ───────────────────────────────────────────────────────

/// Creates the `_migrations` tracking table.
///
/// This is the only place `IF NOT EXISTS` is used - the tracking table must
/// bootstrap itself before version checking can begin. All other migrations
/// use strict DDL so failures are loud.
fn bootstrap_tracking_table(conn: &mut SqliteConnection) -> Result<(), Report<SessionStoreError>> {
    sql_query(
        "CREATE TABLE IF NOT EXISTS _migrations (\
         version INTEGER NOT NULL,\
         name TEXT NOT NULL,\
         applied_at TEXT NOT NULL DEFAULT (datetime('now')))",
    )
    .execute(conn)
    .change_context(SessionStoreError)
    .attach("failed to create _migrations table")?;
    Ok(())
}

/// Row returned by the version query.
#[derive(QueryableByName)]
struct VersionRow {
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
    version: Option<i32>,
}

/// Row returned by `PRAGMA foreign_key_check` (any row = a violation).
///
/// Only `table` is bound; the runner only needs to detect whether *any*
/// row exists. `foreign_key_check` columns: `table`, `rowid`, `parent`, `fkid`.
#[derive(QueryableByName)]
struct FkCheckRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    table: String,
}

/// Reads the highest migration version from the tracking table.
///
/// Returns -1 if no migrations have been recorded (empty database).
fn current_version(conn: &mut SqliteConnection) -> Result<i32, Report<SessionStoreError>> {
    let result: Vec<VersionRow> = sql_query("SELECT MAX(version) AS version FROM _migrations")
        .load(conn)
        .change_context(SessionStoreError)
        .attach("failed to query migration version")?;
    Ok(result.first().and_then(|r| r.version).unwrap_or(-1))
}

/// Records a completed migration in the tracking table.
fn record_version(
    conn: &mut SqliteConnection,
    version: i32,
    name: &str,
) -> Result<(), Report<SessionStoreError>> {
    sql_query("INSERT INTO _migrations (version, name) VALUES (?, ?)")
        .bind::<diesel::sql_types::Integer, _>(version)
        .bind::<diesel::sql_types::Text, _>(name)
        .execute(conn)
        .change_context(SessionStoreError)
        .attach(format!("failed to record migration v{version}"))?;
    Ok(())
}

// ── Migrations ───────────────────────────────────────────────────────────

/// v0: Initial schema - sessions, entries, session_entries, token_ledger.
fn migrate_v0(conn: &mut SqliteConnection) -> Result<(), Report<SessionStoreError>> {
    sql_query(
        "CREATE TABLE sessions (\
         id TEXT PRIMARY KEY,\
         title TEXT,\
         updated_at TEXT NOT NULL,\
         profile TEXT NOT NULL DEFAULT '{}',\
         blobs TEXT NOT NULL DEFAULT '{}',\
         parent_session TEXT DEFAULT NULL,\
         strategy_state TEXT NOT NULL DEFAULT '{}')",
    )
    .execute(conn)
    .change_context(SessionStoreError)
    .attach("v0: create sessions table")?;

    sql_query(
        "CREATE TABLE entries (\
         id TEXT PRIMARY KEY,\
         timestamp TEXT NOT NULL,\
         kind TEXT NOT NULL)",
    )
    .execute(conn)
    .change_context(SessionStoreError)
    .attach("v0: create entries table")?;

    sql_query(
        "CREATE TABLE session_entries (\
         session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,\
         entry_id TEXT NOT NULL REFERENCES entries(id) ON DELETE CASCADE,\
         ordinal INTEGER NOT NULL,\
         pin_position TEXT DEFAULT NULL,\
         PRIMARY KEY (session_id, entry_id),\
         UNIQUE (session_id, ordinal))",
    )
    .execute(conn)
    .change_context(SessionStoreError)
    .attach("v0: create session_entries table")?;

    sql_query(
        "CREATE TABLE token_ledger (\
         id INTEGER PRIMARY KEY AUTOINCREMENT,\
         session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,\
         timestamp TEXT NOT NULL,\
         tokens_sent INTEGER NOT NULL,\
         tokens_received INTEGER NOT NULL)",
    )
    .execute(conn)
    .change_context(SessionStoreError)
    .attach("v0: create token_ledger table")?;

    sql_query("CREATE INDEX idx_session_entries_session ON session_entries(session_id, ordinal)")
        .execute(conn)
        .change_context(SessionStoreError)
        .attach("v0: create session_entries index")?;

    sql_query("CREATE INDEX idx_token_ledger_session ON token_ledger(session_id)")
        .execute(conn)
        .change_context(SessionStoreError)
        .attach("v0: create token_ledger index")?;
    Ok(())
}

/// v1: Add `cwd` column to sessions.
fn migrate_v1(conn: &mut SqliteConnection) -> Result<(), Report<SessionStoreError>> {
    sql_query("ALTER TABLE sessions ADD COLUMN cwd TEXT NOT NULL DEFAULT '.'")
        .execute(conn)
        .change_context(SessionStoreError)
        .attach("v1: add cwd column to sessions")?;
    Ok(())
}

/// v2: Add `created_at` column to sessions.
fn migrate_v2(conn: &mut SqliteConnection) -> Result<(), Report<SessionStoreError>> {
    sql_query("ALTER TABLE sessions ADD COLUMN created_at TEXT NOT NULL DEFAULT ''")
        .execute(conn)
        .change_context(SessionStoreError)
        .attach("v2: add created_at column to sessions")?;
    Ok(())
}

/// v3: Add `ignored` column to session_entries.
///
/// Compaction marks entries as ignored when they've been summarized.
/// Default is `false` (entry is active and visible during prompt assembly).
fn migrate_v3(conn: &mut SqliteConnection) -> Result<(), Report<SessionStoreError>> {
    sql_query("ALTER TABLE session_entries ADD COLUMN ignored BOOLEAN NOT NULL DEFAULT FALSE")
        .execute(conn)
        .change_context(SessionStoreError)
        .attach("v3: add ignored column to session_entries")?;
    Ok(())
}

/// v4: Add `cost` column to token_ledger.
///
/// Tracks per-request cost in USD as reported by the provider (e.g. OpenRouter).
fn migrate_v4(conn: &mut SqliteConnection) -> Result<(), Report<SessionStoreError>> {
    sql_query("ALTER TABLE token_ledger ADD COLUMN cost DOUBLE")
        .execute(conn)
        .change_context(SessionStoreError)
        .attach("v4: add cost column to token_ledger")?;
    Ok(())
}

/// v5: Add \`lifecycle_name\` and \`lifecycle_args\` columns to sessions.
///
/// \`lifecycle_name\` is NULL for sessions created without a lifecycle.
/// \`lifecycle_args\` is a JSON array of strings, defaulting to empty.
fn migrate_v5(conn: &mut SqliteConnection) -> Result<(), Report<SessionStoreError>> {
    sql_query("ALTER TABLE sessions ADD COLUMN lifecycle_name TEXT DEFAULT NULL")
        .execute(conn)
        .change_context(SessionStoreError)
        .attach("v5: add lifecycle_name column to sessions")?;
    sql_query("ALTER TABLE sessions ADD COLUMN lifecycle_args TEXT NOT NULL DEFAULT '[]'")
        .execute(conn)
        .change_context(SessionStoreError)
        .attach("v5: add lifecycle_args column to sessions")?;
    Ok(())
}

/// v6: Add \`archived\` column to sessions.
///
/// Sessions default to unarchived. Closing a session sets \`archived = TRUE\`.
/// On startup, only unarchived sessions are loaded into memory.
#[expect(clippy::expect_used, reason = "infallible")]
fn migrate_v6(conn: &mut SqliteConnection) {
    sql_query("ALTER TABLE sessions ADD COLUMN archived BOOLEAN NOT NULL DEFAULT FALSE")
        .execute(conn)
        .expect("v6: add archived column to sessions");
}

/// v7: Add \`lifecycle_script_state\` column to sessions.
///
/// Persists the [`LifecycleScriptState`] enum so teardown runs correctly
/// after app restart for sessions that had setup run.
/// Default is `'nothing_ran'` - matching the enum's default.
fn migrate_v7(conn: &mut SqliteConnection) -> Result<(), Report<SessionStoreError>> {
    sql_query(
        "ALTER TABLE sessions ADD COLUMN lifecycle_script_state TEXT NOT NULL DEFAULT 'nothing_ran'",
    )
    .execute(conn)
    .change_context(SessionStoreError)
    .attach("v7: add lifecycle_script_state column to sessions")?;
    Ok(())
}

/// v8: Add \`metadata\` column to sessions.
///
/// Stores a JSON blob of all session metadata. This eliminates the need
/// for individual columns per field - new fields on `SessionCore` are
/// automatically persisted via serde.
fn migrate_v8(conn: &mut SqliteConnection) -> Result<(), Report<SessionStoreError>> {
    sql_query("ALTER TABLE sessions ADD COLUMN metadata TEXT")
        .execute(conn)
        .change_context(SessionStoreError)
        .attach("v8: add metadata column to sessions")?;
    Ok(())
}

/// v9: Rename `session_entries` table to `session_history`.
///
/// The old name was ambiguous - it sounded like a table of sessions.
/// The new name makes it clear this is the chat history junction table.
fn migrate_v9(conn: &mut SqliteConnection) -> Result<(), Report<SessionStoreError>> {
    sql_query("ALTER TABLE session_entries RENAME TO session_history")
        .execute(conn)
        .change_context(SessionStoreError)
        .attach("v9: rename session_entries to session_history")?;
    Ok(())
}

/// v10: Consolidate all sessions to compaction-only strategy.
///
/// Rewrites `strategy_state` JSON to remove non-compaction keys
/// and sets `profile.strategy` to `"compaction"` for all sessions.
/// This prepares the database for the removal of other strategy types
/// from the Rust codebase.
fn migrate_v10(conn: &mut SqliteConnection) -> Result<(), Report<SessionStoreError>> {
    #[derive(QueryableByName)]
    struct SessionRow {
        #[diesel(sql_type = diesel::sql_types::Integer)]
        rowid: i32,
        #[diesel(sql_type = diesel::sql_types::Text)]
        strategy_state: String,
        #[diesel(sql_type = diesel::sql_types::Text)]
        profile: String,
    }

    let rows: Vec<SessionRow> = sql_query("SELECT rowid, strategy_state, profile FROM sessions")
        .load(conn)
        .change_context(SessionStoreError)
        .attach("v10: query sessions")?;

    for row in rows {
        let new_strategy_state = rewrite_strategy_state(&row.strategy_state);
        let new_profile = rewrite_profile_strategy(&row.profile);

        sql_query("UPDATE sessions SET strategy_state = ?, profile = ? WHERE rowid = ?")
            .bind::<diesel::sql_types::Text, _>(&new_strategy_state)
            .bind::<diesel::sql_types::Text, _>(&new_profile)
            .bind::<diesel::sql_types::Integer, _>(&row.rowid)
            .execute(conn)
            .change_context(SessionStoreError)
            .attach("v10: update session row")?;
    }

    Ok(())
}

/// Rewrites strategy_state JSON, keeping only the "compaction" key.
///
/// If no compaction key exists, inserts a default.
/// Leaves unparseable JSON unchanged.
fn rewrite_strategy_state(raw: &str) -> String {
    let mut map: std::collections::HashMap<String, serde_json::Value> =
        match serde_json::from_str(raw) {
            Ok(m) => m,
            Err(_) => return raw.to_owned(),
        };

    // Retain only the compaction key.
    map.retain(|k, _| k == "compaction");

    // If compaction key is missing, insert default.
    if !map.contains_key("compaction") {
        map.insert(
            "compaction".to_owned(),
            serde_json::json!({"compaction": {"compaction_count": 0}}),
        );
    }

    serde_json::to_string(&map).unwrap_or_else(|_| raw.to_owned())
}

/// Rewrites profile JSON, setting strategy to "compaction".
///
/// Leaves unparseable JSON unchanged.
fn rewrite_profile_strategy(raw: &str) -> String {
    let mut map: serde_json::Map<String, serde_json::Value> = match serde_json::from_str(raw) {
        Ok(serde_json::Value::Object(m)) => m,
        _ => return raw.to_owned(),
    };

    map.insert(
        "strategy".to_owned(),
        serde_json::Value::String("compaction".to_owned()),
    );

    serde_json::to_string(&serde_json::Value::Object(map)).unwrap_or_else(|_| raw.to_owned())
}

/// Rewrites profile JSON `model` field from bare string to tagged object.
///
/// Before: `"model": "ollama/llama3"`
/// After:  `"model": {"single": "ollama/llama3"}`
///
/// If the `model` field is already an object (already migrated), leaves it unchanged.
/// If the profile JSON is unparseable, leaves it unchanged.
fn rewrite_profile_model(raw: &str) -> String {
    let mut map: serde_json::Map<String, serde_json::Value> = match serde_json::from_str(raw) {
        Ok(serde_json::Value::Object(m)) => m,
        _ => return raw.to_owned(),
    };

    // Only rewrite if `model` is a bare string.
    let Some(model_value) = map.get("model") else {
        return raw.to_owned();
    };
    if let serde_json::Value::String(s) = model_value {
        map.insert("model".to_owned(), serde_json::json!({"single": s}));
    }
    // If model is already an object or missing, leave it alone.

    serde_json::to_string(&serde_json::Value::Object(map)).unwrap_or_else(|_| raw.to_owned())
}

/// v11: Add `is_workflow` column to sessions.
///
/// Marks sessions created by workflow LLM nodes. Enables filtering
/// workflow sessions in the sidebar and elsewhere.
/// Default is `false` - regular chat sessions are not workflow sessions.
fn migrate_v11(conn: &mut SqliteConnection) -> Result<(), Report<SessionStoreError>> {
    sql_query("ALTER TABLE sessions ADD COLUMN is_workflow BOOLEAN NOT NULL DEFAULT FALSE")
        .execute(conn)
        .change_context(SessionStoreError)
        .attach("v11: add is_workflow column to sessions")?;
    Ok(())
}

/// v12: Add `context_override` column to session_history.
///
/// Replaces the boolean `ignored` column with a tri-state text column:
/// `'default'`, `'forced_include'`, `'forced_exclude'`. The old `ignored`
/// column is kept for backward compatibility - the new column takes precedence.
/// Rows with `ignored = 1` are migrated to `'forced_exclude'`.
/// v13: Add `judge_meta` column to sessions.
///
/// Stores judge metadata as a nullable JSON text blob.
/// When NULL, the session is not a judge session.
fn migrate_v13(conn: &mut SqliteConnection) -> Result<(), Report<SessionStoreError>> {
    sql_query("ALTER TABLE sessions ADD COLUMN judge_meta TEXT")
        .execute(conn)
        .change_context(SessionStoreError)
        .attach("v13: add judge_meta column to sessions")?;
    Ok(())
}

/// v14: Add `context_history` column to entries.
///
/// Stores the audit trail of context inclusion/exclusion changes as a JSON array
/// of `ContextChangeEvent`. Defaults to `'[]'` (empty audit) for existing rows.
fn migrate_v14(conn: &mut SqliteConnection) -> Result<(), Report<SessionStoreError>> {
    sql_query("ALTER TABLE entries ADD COLUMN context_history TEXT NOT NULL DEFAULT '[]'")
        .execute(conn)
        .change_context(SessionStoreError)
        .attach("v14: add context_history column to entries")?;
    Ok(())
}

/// Drops the now-unused `strategy_state` column from `sessions`.
/// Context-management strategies were replaced by the history-worker architecture
/// (auto-prune and compaction workers derive state at runtime). The column feeds no
/// code path and is removed to align the schema with the Rust types.
///
/// Implemented via the SQLite-recommended 12-step table rebuild rather than
/// `ALTER TABLE ... DROP COLUMN`, because the latter requires SQLite >= 3.35 and
/// the linked (system) SQLite version is not pinned. The rebuild works on any version.
fn migrate_v15(conn: &mut SqliteConnection) -> Result<(), Report<SessionStoreError>> {
    sql_query(
        "CREATE TABLE sessions_new (\
         id TEXT PRIMARY KEY,\
         title TEXT,\
         updated_at TEXT NOT NULL,\
         profile TEXT NOT NULL DEFAULT '{}',\
         blobs TEXT NOT NULL DEFAULT '{}',\
         parent_session TEXT DEFAULT NULL,\
         cwd TEXT NOT NULL DEFAULT '.',\
         created_at TEXT NOT NULL DEFAULT '',\
         archived BOOLEAN NOT NULL DEFAULT FALSE,\
         lifecycle_name TEXT DEFAULT NULL,\
         lifecycle_args TEXT NOT NULL DEFAULT '[]',\
         lifecycle_script_state TEXT NOT NULL DEFAULT 'nothing_ran',\
         metadata TEXT,\
         is_workflow BOOLEAN NOT NULL DEFAULT FALSE,\
         judge_meta TEXT)",
    )
    .execute(conn)
    .change_context(SessionStoreError)
    .attach("v15: create sessions_new without strategy_state")?;

    sql_query(
        "INSERT INTO sessions_new (\
         id, title, updated_at, profile, blobs, parent_session, cwd, created_at,\
         archived, lifecycle_name, lifecycle_args, lifecycle_script_state, metadata,\
         is_workflow, judge_meta) \
         SELECT \
         id, title, updated_at, profile, blobs, parent_session, cwd, created_at,\
         archived, lifecycle_name, lifecycle_args, lifecycle_script_state, metadata,\
         is_workflow, judge_meta FROM sessions",
    )
    .execute(conn)
    .change_context(SessionStoreError)
    .attach("v15: copy sessions into sessions_new")?;

    sql_query("DROP TABLE sessions")
        .execute(conn)
        .change_context(SessionStoreError)
        .attach("v15: drop old sessions table")?;

    sql_query("ALTER TABLE sessions_new RENAME TO sessions")
        .execute(conn)
        .change_context(SessionStoreError)
        .attach("v15: rename sessions_new to sessions")?;

    Ok(())
}

fn migrate_v16(conn: &mut SqliteConnection) -> Result<(), Report<SessionStoreError>> {
    sql_query(
        "CREATE TABLE sessions_new (\
         id TEXT PRIMARY KEY,\
         title TEXT,\
         updated_at TEXT NOT NULL,\
         profile TEXT NOT NULL DEFAULT '{}',\
         blobs TEXT NOT NULL DEFAULT '{}',\
         parent_session TEXT DEFAULT NULL,\
         cwd TEXT NOT NULL DEFAULT '.',\
         created_at TEXT NOT NULL DEFAULT '',\
         archived BOOLEAN NOT NULL DEFAULT FALSE,\
         lifecycle_name TEXT DEFAULT NULL,\
         lifecycle_args TEXT NOT NULL DEFAULT '[]',\
         lifecycle_script_state TEXT NOT NULL DEFAULT 'nothing_ran',\
         metadata TEXT,\
         is_automated BOOLEAN NOT NULL DEFAULT FALSE,\
         persist BOOLEAN NOT NULL DEFAULT TRUE,\
         judge_meta TEXT)",
    )
    .execute(conn)
    .change_context(SessionStoreError)
    .attach("v16: create sessions_new with is_automated + persist")?;

    sql_query(
        "INSERT INTO sessions_new (\
         id, title, updated_at, profile, blobs, parent_session, cwd, created_at,\
         archived, lifecycle_name, lifecycle_args, lifecycle_script_state, metadata,\
         is_automated, persist, judge_meta) \
         SELECT \
         id, title, updated_at, profile, blobs, parent_session, cwd, created_at,\
         archived, lifecycle_name, lifecycle_args, lifecycle_script_state, metadata,\
         is_workflow, TRUE, judge_meta FROM sessions",
    )
    .execute(conn)
    .change_context(SessionStoreError)
    .attach("v16: copy sessions (is_workflow→is_automated, persist=TRUE)")?;

    sql_query("DROP TABLE sessions")
        .execute(conn)
        .change_context(SessionStoreError)
        .attach("v16: drop old sessions table")?;

    sql_query("ALTER TABLE sessions_new RENAME TO sessions")
        .execute(conn)
        .change_context(SessionStoreError)
        .attach("v16: rename sessions_new to sessions")?;

    Ok(())
}

/// v17: Rewrite profile JSON `model` field from bare string to tagged object,
/// and add `model_used` column to `token_ledger`.
///
/// Before: `"model": "ollama/llama3"`
/// After:  `"model": {"single": "ollama/llama3"}`
///
/// This enables the `ModelSelection` enum to deserialize correctly.
/// Unparseable JSON is left unchanged.
fn migrate_v17(conn: &mut SqliteConnection) -> Result<(), Report<SessionStoreError>> {
    #[derive(QueryableByName)]
    struct SessionRow {
        #[diesel(sql_type = diesel::sql_types::Integer)]
        rowid: i32,
        #[diesel(sql_type = diesel::sql_types::Text)]
        profile: String,
    }

    let rows: Vec<SessionRow> = sql_query("SELECT rowid, profile FROM sessions")
        .load(conn)
        .change_context(SessionStoreError)
        .attach("v17: query sessions")?;

    for row in rows {
        let new_profile = rewrite_profile_model(&row.profile);

        sql_query("UPDATE sessions SET profile = ? WHERE rowid = ?")
            .bind::<diesel::sql_types::Text, _>(&new_profile)
            .bind::<diesel::sql_types::Integer, _>(&row.rowid)
            .execute(conn)
            .change_context(SessionStoreError)
            .attach("v17: update session profile")?;
    }

    let add_result = sql_query("ALTER TABLE token_ledger ADD COLUMN model_used TEXT").execute(conn);

    // Idempotent: ignore "duplicate column" error if migration runs twice.
    match add_result {
        Err(diesel::result::Error::DatabaseError(
            diesel::result::DatabaseErrorKind::Unknown,
            msg,
        )) if msg.message().contains("duplicate column name") => {}
        Err(e) => {
            return Err(e)
                .change_context(SessionStoreError)
                .attach("v17: add model_used column to token_ledger");
        }
        _ => {}
    }

    Ok(())
}


/// v18: Rename `entries.timestamp` column to `timing`.
///
/// The column now stores `EntryTiming` JSON (instant or streamed timing data)
/// rather than a plain timestamp string. The rename aligns the column name
/// with the Rust field it maps to.
fn migrate_v18(conn: &mut SqliteConnection) -> Result<(), Report<SessionStoreError>> {
    sql_query("ALTER TABLE entries RENAME COLUMN timestamp TO timing")
        .execute(conn)
        .change_context(SessionStoreError)
        .attach("v18: rename entries.timestamp to timing")?;
    Ok(())
}
fn migrate_v12(conn: &mut SqliteConnection) -> Result<(), Report<SessionStoreError>> {
    sql_query(
        "ALTER TABLE session_history ADD COLUMN context_override TEXT NOT NULL DEFAULT 'default'",
    )
    .execute(conn)
    .change_context(SessionStoreError)
    .attach("v12: add context_override column to session_history")?;
    sql_query("UPDATE session_history SET context_override = 'forced_exclude' WHERE ignored = 1")
        .execute(conn)
        .change_context(SessionStoreError)
        .attach("v12: migrate ignored values to context_override")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;
    use tempfile::TempDir;

    fn make_conn() -> (TempDir, SqliteConnection) {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("test.db");
        let url = path.to_string_lossy().to_string();
        let mut conn = SqliteConnection::establish(&url).expect("connect");
        sql_query("PRAGMA journal_mode=WAL")
            .execute(&mut conn)
            .expect("WAL");
        (dir, conn)
    }

    #[test]
    fn run_migrations_creates_tracking_table() {
        #[derive(QueryableByName)]
        struct MigrationRow {
            #[diesel(sql_type = diesel::sql_types::Integer)]
            version: i32,
            #[diesel(sql_type = diesel::sql_types::Text)]
            name: String,
        }

        // Given a fresh database.
        let (_dir, mut conn) = make_conn();

        // When running migrations.
        run_migrations(&mut conn).unwrap();

        // Then the _migrations table has 3 entries.

        let rows: Vec<MigrationRow> =
            sql_query("SELECT version, name FROM _migrations ORDER BY version")
                .load(&mut conn)
                .expect("query migrations");

        assert_eq!(rows.len(), 19);
        assert_eq!(rows[0].version, 0);
        assert_eq!(rows[0].name, "create_initial_schema");
        assert_eq!(rows[1].version, 1);
        assert_eq!(rows[1].name, "add_cwd_column");
        assert_eq!(rows[2].version, 2);
        assert_eq!(rows[2].name, "add_created_at_column");
        assert_eq!(rows[3].version, 3);
        assert_eq!(rows[3].name, "add_ignored_to_session_entries");
        assert_eq!(rows[4].version, 4);
        assert_eq!(rows[4].name, "add_cost_to_token_ledger");
        assert_eq!(rows[5].version, 5);
        assert_eq!(rows[5].name, "add_lifecycle_columns_to_sessions");
        assert_eq!(rows[6].version, 6);
        assert_eq!(rows[6].name, "add_archived_column");
        assert_eq!(rows[7].version, 7);
        assert_eq!(rows[7].name, "add_lifecycle_script_state_column");
        assert_eq!(rows[8].version, 8);
        assert_eq!(rows[8].name, "add_metadata_column");
        assert_eq!(rows[9].version, 9);
        assert_eq!(rows[9].name, "rename_session_entries_to_session_history");
        assert_eq!(rows[10].version, 10);
        assert_eq!(rows[10].name, "consolidate_to_compaction_strategy");
        assert_eq!(rows[11].version, 11);
        assert_eq!(rows[11].name, "add_is_workflow_column");
        assert_eq!(rows[12].version, 12);
        assert_eq!(rows[12].name, "replace_ignored_with_context_override");
        assert_eq!(rows[13].version, 13);
        assert_eq!(rows[13].name, "add_judge_meta_column");
        assert_eq!(rows[14].version, 14);
        assert_eq!(rows[14].name, "add_context_history");
    }

    #[test]
    fn re_running_migrations_is_noop() {
        #[derive(QueryableByName)]
        struct CountRow {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            count: i64,
        }

        // Given a database with migrations already applied.
        let (_dir, mut conn) = make_conn();
        run_migrations(&mut conn).unwrap();

        // When running migrations again.
        run_migrations(&mut conn).unwrap();

        // Then no duplicate entries are added.

        let rows: Vec<CountRow> = sql_query("SELECT COUNT(*) AS count FROM _migrations")
            .load(&mut conn)
            .expect("query count");

        assert_eq!(rows[0].count, 19);
    }

    /// Applies migrations up to (and including) `target` version.
    ///
    /// Calls individual migration functions in order, recording each.
    /// This creates a database at exactly version `target` without
    /// applying later migrations.
    fn apply_migrations_up_to(conn: &mut SqliteConnection, target: i32) {
        bootstrap_tracking_table(conn).expect("bootstrap");
        if target >= 0 {
            migrate_v0(conn).expect("v0");
            record_version(conn, 0, "create_initial_schema").expect("record v0");
        }
        if target >= 1 {
            migrate_v1(conn).expect("v1");
            record_version(conn, 1, "add_cwd_column").expect("record v1");
        }
        if target >= 2 {
            migrate_v2(conn).expect("v2");
            record_version(conn, 2, "add_created_at_column").expect("record v2");
        }
        if target >= 3 {
            migrate_v3(conn).expect("v3");
            record_version(conn, 3, "add_ignored_to_session_entries").expect("record v3");
        }
        if target >= 4 {
            migrate_v4(conn).expect("v4");
            record_version(conn, 4, "add_cost_to_token_ledger").expect("record v4");
        }
        if target >= 5 {
            migrate_v5(conn).expect("v5");
            record_version(conn, 5, "add_lifecycle_columns_to_sessions").expect("record v5");
        }
        if target >= 6 {
            migrate_v6(conn);
            record_version(conn, 6, "add_archived_column").expect("record v6");
        }
        if target >= 7 {
            migrate_v7(conn).expect("v7");
            record_version(conn, 7, "add_lifecycle_script_state_column").expect("record v7");
        }
        if target >= 8 {
            migrate_v8(conn).expect("v8");
            record_version(conn, 8, "add_metadata_column").expect("record v8");
        }
        if target >= 9 {
            migrate_v9(conn).expect("v9");
            record_version(conn, 9, "rename_session_entries_to_session_history")
                .expect("record v9");
        }
        if target >= 10 {
            migrate_v10(conn).expect("v10");
            record_version(conn, 10, "consolidate_to_compaction_strategy").expect("record v10");
        }
        if target >= 11 {
            migrate_v11(conn).expect("v11");
            record_version(conn, 11, "add_is_workflow_column").expect("record v11");
        }
        if target >= 12 {
            migrate_v12(conn).expect("v12");
            record_version(conn, 12, "replace_ignored_with_context_override").expect("record v12");
        }
        if target >= 13 {
            migrate_v13(conn).expect("v13");
            record_version(conn, 13, "add_judge_meta_column").expect("record v13");
        }
        if target >= 14 {
            migrate_v14(conn).expect("v14");
            record_version(conn, 14, "add_context_history").expect("record v14");
        }
        if target >= 15 {
            migrate_v15(conn).expect("v15");
            record_version(conn, 15, "drop_strategy_state_column").expect("record v15");
        }
        if target >= 16 {
            migrate_v16(conn).expect("v16");
            record_version(
                conn,
                16,
                "rename_is_workflow_to_is_automated_and_add_persist",
            )
            .expect("record v16");
        }
        if target >= 17 {
            migrate_v17(conn).expect("v17");
            record_version(
                conn,
                17,
                "rewrite_model_to_model_selection_and_add_model_used",
            )
            .expect("record v17");
        }
    }

    /// Verifies that each migration guard uses `<` not `<=`.
    ///
    /// For each version N (0..=13), we build a database at exactly version N
    /// by calling individual migration functions, then re-run `run_migrations`.
    /// It must succeed (applying only v(N+1) through v14) and produce exactly
    /// 15 migration rows.
    ///
    /// If `current < N` were mutated to `current <= N`, vN would re-run when
    /// current == N. Most migrations would fail (duplicate table/column),
    /// and those that don't fail would produce a duplicate _migrations row,
    /// causing the count assertion to fail.
    #[test]
    fn migration_guards_do_not_reapply_completed_version() {
        #[derive(QueryableByName)]
        struct CountRow {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            count: i64,
        }

        for target_version in 0..=17_i32 {
            let (_dir, mut conn) = make_conn();

            // Build the database at exactly `target_version`.
            apply_migrations_up_to(&mut conn, target_version);

            // Re-running should succeed - applying only versions > target_version.
            run_migrations(&mut conn).unwrap_or_else(|e| {
                panic!("re-run at target_version={target_version} should succeed: {e:?}")
            });

            // Verify no duplicate rows: exactly 18 migration rows total.
            let rows: Vec<CountRow> = sql_query("SELECT COUNT(*) AS count FROM _migrations")
                .load(&mut conn)
                .expect("query count");
            assert_eq!(
                rows[0].count, 19,
                "at target_version={target_version}: expected 19 migration rows, no duplicates"
            );
        }
    }

    #[test]
    fn fresh_database_has_all_tables() {
        #[derive(QueryableByName)]
        struct TableRow {
            #[diesel(sql_type = diesel::sql_types::Text)]
            name: String,
        }

        // Given a fresh database.
        let (_dir, mut conn) = make_conn();

        // When running migrations.
        run_migrations(&mut conn).unwrap();

        // Then all expected tables exist.

        let tables: Vec<TableRow> =
            sql_query("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                .load(&mut conn)
                .expect("query tables");
        let table_names: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();

        assert!(table_names.contains(&"_migrations"));
        assert!(table_names.contains(&"entries"));
        assert!(table_names.contains(&"session_history"));
        assert!(table_names.contains(&"sessions"));
        assert!(table_names.contains(&"token_ledger"));
    }

    #[test]
    fn migrate_v10_consolidates_strategy_state() {
        #[derive(QueryableByName)]
        struct StateRow {
            #[diesel(sql_type = diesel::sql_types::Text)]
            strategy_state: String,
            #[diesel(sql_type = diesel::sql_types::Text)]
            profile: String,
        }

        // Given a database with sessions containing mixed strategy state and profile.
        let (_dir, mut conn) = make_conn();
        apply_migrations_up_to(&mut conn, 9); // build at v9, before v10 rewrites strategy_state

        // Insert a session with passthrough strategy state and sliding_window profile strategy.
        sql_query(
            "INSERT INTO sessions (id, title, updated_at, created_at, cwd, profile, strategy_state, blobs, lifecycle_script_state) \
             VALUES ('test-1', 'Test', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z', '.', \
             '{\"strategy\": \"sliding_window\", \"model\": \"test\", \"persona_name\": \"coding-assistant\", \"token_budget\": 150000, \"sliding_window_size\": 5}', \
             '{\"passthrough\": \"Passthrough\", \"compaction\": {\"compaction\": {\"compaction_count\": 2}}}', \
             '{}', 'nothing_ran')",
        )
        .execute(&mut conn)
        .expect("insert test session");

        // When running migration v10.
        migrate_v10(&mut conn).expect("migrate v10");

        // Then strategy_state only has the compaction key.
        let rows: Vec<StateRow> =
            sql_query("SELECT strategy_state, profile FROM sessions WHERE id = 'test-1'")
                .load(&mut conn)
                .expect("query");

        assert_eq!(rows.len(), 1);

        let state_map: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&rows[0].strategy_state).expect("parse state");
        assert!(state_map.contains_key("compaction"));
        assert!(!state_map.contains_key("passthrough"));
        assert!(!state_map.contains_key("sliding_window"));
        assert!(!state_map.contains_key("token_budget"));

        // And profile.strategy is "compaction".
        let profile: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&rows[0].profile).expect("parse profile");
        assert_eq!(profile["strategy"], "compaction");
    }

    #[test]
    fn migrate_v10_inserts_default_when_no_compaction_key() {
        #[derive(QueryableByName)]
        struct StateRow {
            #[diesel(sql_type = diesel::sql_types::Text)]
            strategy_state: String,
        }

        // Given a database with a session having no compaction key in strategy_state.
        let (_dir, mut conn) = make_conn();
        apply_migrations_up_to(&mut conn, 9); // build at v9, before v10 rewrites strategy_state

        sql_query(
            "INSERT INTO sessions (id, title, updated_at, created_at, cwd, profile, strategy_state, blobs, lifecycle_script_state) \
             VALUES ('test-2', 'Test', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z', '.', \
             '{\"strategy\": \"passthrough\", \"model\": \"test\"}', \
             '{\"passthrough\": \"Passthrough\"}', \
             '{}', 'nothing_ran')",
        )
        .execute(&mut conn)
        .expect("insert test session");

        // When running migration v10.
        migrate_v10(&mut conn).expect("migrate v10");

        // Then strategy_state has a compaction key with default data.
        let rows: Vec<StateRow> =
            sql_query("SELECT strategy_state FROM sessions WHERE id = 'test-2'")
                .load(&mut conn)
                .expect("query");

        assert_eq!(rows.len(), 1);
        let state_map: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&rows[0].strategy_state).expect("parse state");
        assert!(state_map.contains_key("compaction"));
        assert!(!state_map.contains_key("passthrough"));
    }

    #[test]
    fn migrate_v15_drops_strategy_state_column() {
        #[derive(QueryableByName)]
        struct ColumnRow {
            #[diesel(sql_type = diesel::sql_types::Text)]
            name: String,
        }

        #[derive(QueryableByName)]
        struct IdRow {
            #[diesel(sql_type = diesel::sql_types::Text)]
            id: String,
        }

        // Given a database built to v14 (strategy_state column still present) with a session in it.
        let (_dir, mut conn) = make_conn();
        apply_migrations_up_to(&mut conn, 14);
        sql_query(
            "INSERT INTO sessions (id, title, updated_at, created_at, cwd, profile, strategy_state, blobs, lifecycle_script_state) \
             VALUES ('keep-me', 'Survivor', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z', '.', \
             '{}', '{}', '{}', 'nothing_ran')",
        )
        .execute(&mut conn)
        .expect("insert pre-v15 session");

        // When running migration v15.
        migrate_v15(&mut conn).expect("migrate v15");

        // Then the sessions table no longer has a strategy_state column.
        let cols: Vec<ColumnRow> = sql_query("PRAGMA table_info(sessions)")
            .load(&mut conn)
            .expect("query table_info");
        assert!(
            !cols.iter().any(|c| c.name == "strategy_state"),
            "strategy_state column should be dropped; found columns: {:?}",
            cols.iter().map(|c| &c.name).collect::<Vec<_>>()
        );

        // And the session row survived the table rebuild.
        let rows: Vec<IdRow> = sql_query("SELECT id FROM sessions")
            .load(&mut conn)
            .expect("query sessions");
        assert!(
            rows.iter().any(|r| r.id == "keep-me"),
            "session 'keep-me' must survive the v15 table rebuild"
        );
    }

    #[test]
    fn v15_rebuild_preserves_session_history_rows() {
        #[derive(QueryableByName)]
        struct CountRow {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            count: i64,
        }

        // Given a v14 database with a session linked to an entry via session_history,
        // and the connection running with foreign_keys=ON — as the production
        // bootstrap connection does.
        let (_dir, mut conn) = make_conn();
        apply_migrations_up_to(&mut conn, 14);
        sql_query("PRAGMA foreign_keys=ON")
            .execute(&mut conn)
            .expect("FK on");
        seed_session_with_children(&mut conn);

        // When running migrations (applies v15, the DROP TABLE sessions rebuild).
        run_migrations(&mut conn).expect("run_migrations");

        // Then the session_history junction row survived the rebuild.
        // Before the FK-safe fix, DROP TABLE sessions cascaded ON DELETE CASCADE
        // into session_history and wiped every junction row.
        let rows: Vec<CountRow> = sql_query("SELECT COUNT(*) AS count FROM session_history")
            .load(&mut conn)
            .expect("count session_history");
        assert_eq!(
            rows[0].count, 1,
            "session_history junction row must survive v15 rebuild"
        );
    }

    #[test]
    fn v15_rebuild_preserves_token_ledger_rows() {
        #[derive(QueryableByName)]
        struct CountRow {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            count: i64,
        }

        // Given a v14 database with a token_ledger row for a session,
        // and the connection running with foreign_keys=ON.
        let (_dir, mut conn) = make_conn();
        apply_migrations_up_to(&mut conn, 14);
        sql_query("PRAGMA foreign_keys=ON")
            .execute(&mut conn)
            .expect("FK on");
        seed_session_with_children(&mut conn);

        // When running migrations (applies v15).
        run_migrations(&mut conn).expect("run_migrations");

        // Then the token_ledger row survived the rebuild.
        // token_ledger also has ON DELETE CASCADE on sessions, so the same
        // DROP TABLE cascade would have wiped it before the fix.
        let rows: Vec<CountRow> = sql_query("SELECT COUNT(*) AS count FROM token_ledger")
            .load(&mut conn)
            .expect("count token_ledger");
        assert_eq!(
            rows[0].count, 1,
            "token_ledger row must survive v15 rebuild"
        );
    }

    /// Seeds a v14 database with one session, one entry, one session_history
    /// junction row linking them, and one token_ledger row.
    ///
    /// Call after `apply_migrations_up_to(conn, 14)`.
    fn seed_session_with_children(conn: &mut SqliteConnection) {
        sql_query(
            "INSERT INTO sessions (id, title, updated_at, created_at, cwd, profile, strategy_state, blobs, lifecycle_script_state) \n
             VALUES ('s-1', 'T', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z', '.', \n
             '{}', '{}', '{}', 'nothing_ran')",
        )
        .execute(conn)
        .expect("seed session");
        sql_query(
            "INSERT INTO entries (id, timestamp, kind) VALUES ('e-1', '2024-01-01T00:00:00Z', '\"User\"')",
        )
        .execute(conn)
        .expect("seed entry");
        sql_query(
            "INSERT INTO session_history (session_id, entry_id, ordinal) VALUES ('s-1', 'e-1', 0)",
        )
        .execute(conn)
        .expect("seed junction");
        sql_query(
            "INSERT INTO token_ledger (session_id, timestamp, tokens_sent, tokens_received) \n
             VALUES ('s-1', '2024-01-01T00:00:00Z', 10, 20)",
        )
        .execute(conn)
        .expect("seed ledger");
    }

    #[test]
    fn run_migrations_preserves_child_rows_and_leaves_fk_clean() {
        #[derive(QueryableByName)]
        struct CountRow {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            count: i64,
        }

        #[derive(QueryableByName)]
        struct FkRow {
            #[diesel(sql_type = diesel::sql_types::Text)]
            table: String,
            #[diesel(sql_type = diesel::sql_types::Text)]
            _rowid: String,
            #[diesel(sql_type = diesel::sql_types::Text)]
            _refer: String,
            #[diesel(sql_type = diesel::sql_types::Text)]
            _parent: String,
        }

        // Given a v14 database with FK=ON and a full session (junction + ledger).
        let (_dir, mut conn) = make_conn();
        apply_migrations_up_to(&mut conn, 14);
        sql_query("PRAGMA foreign_keys=ON")
            .execute(&mut conn)
            .expect("FK on");
        seed_session_with_children(&mut conn);

        // When running the full migration suite.
        run_migrations(&mut conn).expect("run_migrations");

        // Then both child tables retain their row.
        let history: Vec<CountRow> = sql_query("SELECT COUNT(*) AS count FROM session_history")
            .load(&mut conn)
            .expect("count session_history");
        assert_eq!(history[0].count, 1, "session_history row preserved");

        let ledger: Vec<CountRow> = sql_query("SELECT COUNT(*) AS count FROM token_ledger")
            .load(&mut conn)
            .expect("count token_ledger");
        assert_eq!(ledger[0].count, 1, "token_ledger row preserved");

        // And foreign_key_check reports no violations on the final state.
        let violations: Vec<FkRow> = sql_query("PRAGMA foreign_key_check")
            .load(&mut conn)
            .expect("foreign_key_check");
        assert!(
            violations.is_empty(),
            "foreign_key_check must be clean after migrations; found: {:?}",
            violations
                .iter()
                .map(|v| v.table.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn migrate_v17_rewrites_bare_string_model_to_single() {
        #[derive(QueryableByName)]
        struct ProfileRow {
            #[diesel(sql_type = diesel::sql_types::Text)]
            profile: String,
        }

        // Given a database at v16 with a session having a bare-string model.
        let (_dir, mut conn) = make_conn();
        apply_migrations_up_to(&mut conn, 16);
        sql_query(
            "INSERT INTO sessions (id, title, updated_at, created_at, cwd, profile, blobs, lifecycle_script_state, is_automated, persist) \
             VALUES ('test-v17', 'Test', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z', '.', \
             '{\"strategy\": \"sliding_window\", \"model\": \"ollama/llama3\", \"persona_name\": \"coding-assistant\", \"token_budget\": 150000, \"sliding_window_size\": 5}', \
             '{}', 'nothing_ran', 0, 0)"
        )
        .execute(&mut conn)
        .expect("insert pre-v17 session");

        // When running migration v17.
        migrate_v17(&mut conn).expect("migrate v17");

        // Then the profile JSON has model rewritten as {"single":"ollama/llama3"}.
        let rows: Vec<ProfileRow> = sql_query("SELECT profile FROM sessions WHERE id = 'test-v17'")
            .load(&mut conn)
            .expect("query");

        assert_eq!(rows.len(), 1);
        let profile: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&rows[0].profile).expect("parse profile");
        assert_eq!(
            profile["model"],
            serde_json::json!({"single": "ollama/llama3"})
        );
        // And other profile fields are preserved.
        assert_eq!(profile["strategy"], "sliding_window");
        assert_eq!(profile["persona_name"], "coding-assistant");
    }

    #[test]
    fn migrate_v17_handles_no_provider_id() {
        #[derive(QueryableByName)]
        struct ProfileRow {
            #[diesel(sql_type = diesel::sql_types::Text)]
            profile: String,
        }

        // Given a database at v16 with a session having <none> as model.
        let (_dir, mut conn) = make_conn();
        apply_migrations_up_to(&mut conn, 16);
        sql_query(
            "INSERT INTO sessions (id, title, updated_at, created_at, cwd, profile, blobs, lifecycle_script_state, is_automated, persist) \
             VALUES ('test-none', 'Test', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z', '.', \
             '{\"strategy\": \"sliding_window\", \"model\": \"<none>\", \"persona_name\": \"coding-assistant\"}', \
             '{}', 'nothing_ran', 0, 0)"
        )
        .execute(&mut conn)
        .expect("insert pre-v17 session");

        // When running migration v17.
        migrate_v17(&mut conn).expect("migrate v17");

        // Then the profile JSON has model rewritten as {"single":"<none>"}.
        let rows: Vec<ProfileRow> =
            sql_query("SELECT profile FROM sessions WHERE id = 'test-none'")
                .load(&mut conn)
                .expect("query");

        assert_eq!(rows.len(), 1);
        let profile: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&rows[0].profile).expect("parse profile");
        assert_eq!(profile["model"], serde_json::json!({"single": "<none>"}));
    }

    #[test]
    fn migrate_v17_is_idempotent() {
        #[derive(QueryableByName)]
        struct ProfileRow {
            #[diesel(sql_type = diesel::sql_types::Text)]
            profile: String,
        }

        // Given a database at v16 with a session.
        let (_dir, mut conn) = make_conn();
        apply_migrations_up_to(&mut conn, 16);
        sql_query(
            "INSERT INTO sessions (id, title, updated_at, created_at, cwd, profile, blobs, lifecycle_script_state, is_automated, persist) \
             VALUES ('test-idem', 'Test', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z', '.', \
             '{\"strategy\": \"sliding_window\", \"model\": \"ollama/llama3\"}', \
             '{}', 'nothing_ran', 0, 0)"
        )
        .execute(&mut conn)
        .expect("insert pre-v17 session");

        // When running migration v17 twice.
        migrate_v17(&mut conn).expect("migrate v17 first");
        migrate_v17(&mut conn).expect("migrate v17 second");

        // Then the profile is still correct, not double-wrapped.
        let rows: Vec<ProfileRow> =
            sql_query("SELECT profile FROM sessions WHERE id = 'test-idem'")
                .load(&mut conn)
                .expect("query");

        assert_eq!(rows.len(), 1);
        let profile: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&rows[0].profile).expect("parse profile");
        assert_eq!(
            profile["model"],
            serde_json::json!({"single": "ollama/llama3"})
        );
    }

    #[test]
    fn migrate_v17_adds_model_used_column() {
        #[derive(QueryableByName)]
        struct ColumnRow {
            #[diesel(sql_type = diesel::sql_types::Text)]
            name: String,
        }

        // Given a database at v16 (no model_used column).
        let (_dir, mut conn) = make_conn();
        apply_migrations_up_to(&mut conn, 16);

        // When running migration v17.
        migrate_v17(&mut conn).expect("migrate v17");

        // Then the token_ledger table has a model_used column.
        let cols: Vec<ColumnRow> = sql_query("PRAGMA table_info(token_ledger)")
            .load(&mut conn)
            .expect("pragma");
        let col_names: Vec<&str> = cols.iter().map(|c| c.name.as_str()).collect();
        assert!(
            col_names.contains(&"model_used"),
            "expected model_used column, got: {col_names:?}"
        );
    }

    #[test]
    fn migrate_v18_renames_timestamp_and_preserves_value() {
        #[derive(QueryableByName)]
        struct TimingRow {
            #[diesel(sql_type = diesel::sql_types::Text)]
            timing: String,
        }

        // Given a database at v16 (timestamp column, before rename).
        let (_dir, mut conn) = make_conn();
        apply_migrations_up_to(&mut conn, 15);
        migrate_v16(&mut conn).expect("v16");
        record_version(&mut conn, 16, "add_persist_and_is_automated").expect("record v16");

        // Insert a row with the old `timestamp` column.
        let legacy_ts = "2024-01-15T10:30:00Z";
        sql_query("INSERT INTO entries (id, timestamp, kind) VALUES ('e1', ?, 'user')")
            .bind::<diesel::sql_types::Text, _>(legacy_ts)
            .execute(&mut conn)
            .expect("insert legacy row");

        // When running migrations (v18 renames timestamp → timing).
        run_migrations(&mut conn).expect("run_migrations");

        // Then the column is renamed and the value is preserved.
        let rows: Vec<TimingRow> = sql_query("SELECT timing FROM entries WHERE id = 'e1'")
            .load(&mut conn)
            .expect("read timing");
        assert_eq!(rows.len(), 1, "entry should exist");
        assert_eq!(rows[0].timing, legacy_ts, "value preserved through rename");
    }
}
