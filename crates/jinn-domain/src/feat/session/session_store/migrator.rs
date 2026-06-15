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

use dao::Pool;
use error_stack::{Report, ResultExt as _};
use rusqlite::params;
use std::collections::HashMap;

use super::SessionStoreError;

/// Runs all pending database migrations.
///
/// Migrations run with `PRAGMA foreign_keys=OFF`. This is essential: DDL such
/// as `DROP TABLE sessions` (used by table-rebuild migrations like `migrate_v15`
/// or `migrate_v20`) performs an implicit `DELETE` of all rows, which would
/// otherwise fire the application-level `ON DELETE CASCADE` and wipe every
/// `session_history` and `token_ledger` row. After the migrations complete (or
/// fail), FK is re-enabled and `PRAGMA foreign_key_check` verifies referential
/// integrity.
///
/// The entire sequence (FK-off → migrate → FK-on → check) runs inside a single
/// [`dao::Pool::with_conn`] closure so the per-connection `PRAGMA foreign_keys`
/// scoping is preserved: the pragma is a no-op inside an active transaction,
/// and `with_conn` does not open one.
///
/// # Errors
///
/// Returns an error if any migration fails, if the FK pragma cannot be
/// toggled, or if `foreign_key_check` reports integrity violations after
/// the migration run.
pub async fn run_migrations(pool: &Pool) -> Result<(), Report<SessionStoreError>> {
    // The closure returns `dao::Result<Result<_, Report<…>>>`: the outer `dao::Error`
    // covers connection/pragma/fk-check failures; the inner `Result` is the migration
    // outcome (which carries rich `.attach()` context). `change_context` folds the
    // outer layer into `Report<SessionStoreError>` at the boundary.
    let outcome: Result<(), Report<SessionStoreError>> = pool
        .with_conn(
            |conn| -> dao::Result<Result<(), Report<SessionStoreError>>> {
                conn.pragma_update(None, "foreign_keys", "OFF")?;

                let migrate_result = run_pending_migrations(conn);

                // Always re-enable FK + check integrity, even if a migration failed,
                // so the connection is left in its normal (FK-on) state.
                conn.pragma_update(None, "foreign_keys", "ON")?;
                let violations = fk_violations(conn)?;

                match (migrate_result, violations) {
                    (Ok(()), empty) if empty.is_empty() => Ok(Ok(())),
                    (Ok(()), tables) => Ok(Err(Report::new(SessionStoreError)
                        .attach("foreign_key_check reported violations after migration")
                        .attach(format!("violating tables: {}", tables.join(", "))))),
                    (Err(e), _) => Ok(Err(e)),
                }
            },
        )
        .await
        .change_context(SessionStoreError)
        .attach("failed to run migrations")?;

    outcome
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
fn run_pending_migrations(
    conn: &mut rusqlite::Connection,
) -> Result<(), Report<SessionStoreError>> {
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
    if current < 19 {
        migrate_v19(conn)?;
        record_version(conn, 19, "rewrite_metadata_blob_profile_model")?;
    }
    if current < 20 {
        migrate_v20(conn)?;
        record_version(conn, 20, "drop_zombie_columns_backfill_metadata")?;
    }
    Ok(())
}

// ── Tracking table ───────────────────────────────────────────────────────

/// Creates the `_migrations` tracking table.
///
/// This is the only place `IF NOT EXISTS` is used - the tracking table must
/// bootstrap itself before version checking can begin. All other migrations
/// use strict DDL so failures are loud.
fn bootstrap_tracking_table(
    conn: &mut rusqlite::Connection,
) -> Result<(), Report<SessionStoreError>> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _migrations (\
         version INTEGER NOT NULL,\
         name TEXT NOT NULL,\
         applied_at TEXT NOT NULL DEFAULT (datetime('now')))",
    )
    .change_context(SessionStoreError)
    .attach("failed to create _migrations table")?;
    Ok(())
}

/// Reads the highest migration version from the tracking table.
///
/// Returns -1 if no migrations have been recorded (empty database).
fn current_version(conn: &mut rusqlite::Connection) -> Result<i32, Report<SessionStoreError>> {
    let version: Option<i32> = conn
        .query_row(
            "SELECT MAX(version) AS version FROM _migrations",
            [],
            |row| row.get(0),
        )
        .change_context(SessionStoreError)
        .attach("failed to query migration version")?;
    Ok(version.unwrap_or(-1))
}

/// Records a completed migration in the tracking table.
fn record_version(
    conn: &mut rusqlite::Connection,
    version: i32,
    name: &str,
) -> Result<(), Report<SessionStoreError>> {
    conn.execute(
        "INSERT INTO _migrations (version, name) VALUES (?, ?)",
        params![version, name],
    )
    .change_context(SessionStoreError)
    .attach(format!("failed to record migration v{version}"))?;
    Ok(())
}

/// Returns the names of tables that violate foreign-key constraints.
///
/// `PRAGMA foreign_key_check` returns one row per violation (empty = clean).
/// Columns: `table`, `rowid`, `parent`, `fkid` — only `table` is bound, since
/// the runner only needs the names for the error attachment.
/// Returns the names of tables with foreign-key violations.
///
/// Returns `dao::Result` so it composes inside a `with_conn` closure without a
/// double-`Report` wrap.
fn fk_violations(conn: &mut rusqlite::Connection) -> dao::Result<Vec<String>> {
    let mut stmt = conn.prepare("PRAGMA foreign_key_check")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut tables = Vec::new();
    for row in rows {
        tables.push(row?);
    }
    Ok(tables)
}

// ── Migrations ───────────────────────────────────────────────────────────

/// v0: Initial schema - sessions, entries, session_entries, token_ledger.
fn migrate_v0(conn: &mut rusqlite::Connection) -> Result<(), Report<SessionStoreError>> {
    conn.execute_batch(
        "CREATE TABLE sessions (\
         id TEXT PRIMARY KEY,\
         title TEXT,\
         updated_at TEXT NOT NULL,\
         profile TEXT NOT NULL DEFAULT '{}',\
         blobs TEXT NOT NULL DEFAULT '{}',\
         parent_session TEXT DEFAULT NULL,\
         strategy_state TEXT NOT NULL DEFAULT '{}')",
    )
    .change_context(SessionStoreError)
    .attach("v0: create sessions table")?;

    conn.execute_batch(
        "CREATE TABLE entries (\
         id TEXT PRIMARY KEY,\
         timestamp TEXT NOT NULL,\
         kind TEXT NOT NULL)",
    )
    .change_context(SessionStoreError)
    .attach("v0: create entries table")?;

    conn.execute_batch(
        "CREATE TABLE session_entries (\
         session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,\
         entry_id TEXT NOT NULL REFERENCES entries(id) ON DELETE CASCADE,\
         ordinal INTEGER NOT NULL,\
         pin_position TEXT DEFAULT NULL,\
         PRIMARY KEY (session_id, entry_id),\
         UNIQUE (session_id, ordinal))",
    )
    .change_context(SessionStoreError)
    .attach("v0: create session_entries table")?;

    conn.execute_batch(
        "CREATE TABLE token_ledger (\
         id INTEGER PRIMARY KEY AUTOINCREMENT,\
         session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,\
         timestamp TEXT NOT NULL,\
         tokens_sent INTEGER NOT NULL,\
         tokens_received INTEGER NOT NULL)",
    )
    .change_context(SessionStoreError)
    .attach("v0: create token_ledger table")?;

    conn.execute_batch(
        "CREATE INDEX idx_session_entries_session ON session_entries(session_id, ordinal)",
    )
    .change_context(SessionStoreError)
    .attach("v0: create session_entries index")?;

    conn.execute_batch("CREATE INDEX idx_token_ledger_session ON token_ledger(session_id)")
        .change_context(SessionStoreError)
        .attach("v0: create token_ledger index")?;
    Ok(())
}

/// v1: Add `cwd` column to sessions.
fn migrate_v1(conn: &mut rusqlite::Connection) -> Result<(), Report<SessionStoreError>> {
    conn.execute_batch("ALTER TABLE sessions ADD COLUMN cwd TEXT NOT NULL DEFAULT '.'")
        .change_context(SessionStoreError)
        .attach("v1: add cwd column to sessions")?;
    Ok(())
}

/// v2: Add `created_at` column to sessions.
fn migrate_v2(conn: &mut rusqlite::Connection) -> Result<(), Report<SessionStoreError>> {
    conn.execute_batch("ALTER TABLE sessions ADD COLUMN created_at TEXT NOT NULL DEFAULT ''")
        .change_context(SessionStoreError)
        .attach("v2: add created_at column to sessions")?;
    Ok(())
}

/// v3: Add `ignored` column to session_entries.
///
/// Compaction marks entries as ignored when they've been summarized.
/// Default is `false` (entry is active and visible during prompt assembly).
fn migrate_v3(conn: &mut rusqlite::Connection) -> Result<(), Report<SessionStoreError>> {
    conn.execute_batch(
        "ALTER TABLE session_entries ADD COLUMN ignored BOOLEAN NOT NULL DEFAULT FALSE",
    )
    .change_context(SessionStoreError)
    .attach("v3: add ignored column to session_entries")?;
    Ok(())
}

/// v4: Add `cost` column to token_ledger.
///
/// Tracks per-request cost in USD as reported by the provider (e.g. OpenRouter).
fn migrate_v4(conn: &mut rusqlite::Connection) -> Result<(), Report<SessionStoreError>> {
    conn.execute_batch("ALTER TABLE token_ledger ADD COLUMN cost DOUBLE")
        .change_context(SessionStoreError)
        .attach("v4: add cost column to token_ledger")?;
    Ok(())
}

/// v5: Add `lifecycle_name` and `lifecycle_args` columns to sessions.
///
/// `lifecycle_name` is NULL for sessions created without a lifecycle.
/// `lifecycle_args` is a JSON array of strings, defaulting to empty.
fn migrate_v5(conn: &mut rusqlite::Connection) -> Result<(), Report<SessionStoreError>> {
    conn.execute_batch("ALTER TABLE sessions ADD COLUMN lifecycle_name TEXT DEFAULT NULL")
        .change_context(SessionStoreError)
        .attach("v5: add lifecycle_name column to sessions")?;
    conn.execute_batch("ALTER TABLE sessions ADD COLUMN lifecycle_args TEXT NOT NULL DEFAULT '[]'")
        .change_context(SessionStoreError)
        .attach("v5: add lifecycle_args column to sessions")?;
    Ok(())
}

/// v6: Add `archived` column to sessions.
///
/// Sessions default to unarchived. Closing a session sets `archived = TRUE`.
/// On startup, only unarchived sessions are loaded into memory.
#[expect(clippy::expect_used, reason = "infallible")]
fn migrate_v6(conn: &mut rusqlite::Connection) {
    conn.execute_batch("ALTER TABLE sessions ADD COLUMN archived BOOLEAN NOT NULL DEFAULT FALSE")
        .expect("v6: add archived column to sessions");
}

/// v7: Add `lifecycle_script_state` column to sessions.
///
/// Persists the [`LifecycleScriptState`] enum so teardown runs correctly
/// after app restart for sessions that had setup run.
/// Default is `'nothing_ran'` - matching the enum's default.
fn migrate_v7(conn: &mut rusqlite::Connection) -> Result<(), Report<SessionStoreError>> {
    conn.execute_batch(
        "ALTER TABLE sessions ADD COLUMN lifecycle_script_state TEXT NOT NULL DEFAULT 'nothing_ran'",
    )
    .change_context(SessionStoreError)
    .attach("v7: add lifecycle_script_state column to sessions")?;
    Ok(())
}

/// v8: Add `metadata` column to sessions.
///
/// Stores a JSON blob of all session metadata. This eliminates the need
/// for individual columns per field - new fields on `SessionCore` are
/// automatically persisted via serde.
fn migrate_v8(conn: &mut rusqlite::Connection) -> Result<(), Report<SessionStoreError>> {
    conn.execute_batch("ALTER TABLE sessions ADD COLUMN metadata TEXT")
        .change_context(SessionStoreError)
        .attach("v8: add metadata column to sessions")?;
    Ok(())
}

/// v9: Rename `session_entries` table to `session_history`.
///
/// The old name was ambiguous - it sounded like a table of sessions.
/// The new name makes it clear this is the chat history junction table.
fn migrate_v9(conn: &mut rusqlite::Connection) -> Result<(), Report<SessionStoreError>> {
    conn.execute_batch("ALTER TABLE session_entries RENAME TO session_history")
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
fn migrate_v10(conn: &mut rusqlite::Connection) -> Result<(), Report<SessionStoreError>> {
    let rows = query_map_rows(
        conn,
        "SELECT rowid, strategy_state, profile FROM sessions",
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        },
        "v10",
    )?;

    for (rowid, strategy_state, profile) in rows {
        let new_strategy_state = rewrite_strategy_state(&strategy_state);
        let new_profile = rewrite_profile_strategy(&profile);

        conn.execute(
            "UPDATE sessions SET strategy_state = ?, profile = ? WHERE rowid = ?",
            params![new_strategy_state, new_profile, rowid],
        )
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
    let mut map: HashMap<String, serde_json::Value> = match serde_json::from_str(raw) {
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

/// Rewrites a bare-string `profile.model` inside a `metadata` JSON blob.
///
/// v19's analog of [`rewrite_profile_model`], operating one level deeper:
/// the metadata blob embeds a `profile` sub-object, whose `model` field
/// needs the same bare-string -> `{"single": ...}` transformation. This
/// extracts the profile sub-object, hands it to [`rewrite_profile_model`],
/// and re-inserts the result. Idempotent and non-destructive: returns the
/// input unchanged on any parse failure, missing `profile` key,
/// non-object `profile`, or when `model` is already an object/missing.
fn rewrite_metadata_blob_model(raw_metadata: &str) -> String {
    let mut root: serde_json::Map<String, serde_json::Value> =
        match serde_json::from_str(raw_metadata) {
            Ok(serde_json::Value::Object(m)) => m,
            _ => return raw_metadata.to_owned(),
        };

    let Some(profile_value) = root.get("profile") else {
        return raw_metadata.to_owned();
    };
    let serde_json::Value::Object(_) = profile_value else {
        // `profile` present but not an object - malformed; leave it alone.
        return raw_metadata.to_owned();
    };

    // Serialize the profile sub-object, run the model rewriter, parse back.
    let Ok(profile_str) = serde_json::to_string(profile_value) else {
        return raw_metadata.to_owned();
    };
    let new_profile_str = rewrite_profile_model(&profile_str);
    let new_profile: serde_json::Value = match serde_json::from_str(&new_profile_str) {
        Ok(v) => v,
        Err(_) => return raw_metadata.to_owned(),
    };
    root.insert("profile".to_owned(), new_profile);

    serde_json::to_string(&serde_json::Value::Object(root))
        .unwrap_or_else(|_| raw_metadata.to_owned())
}

/// v11: Add `is_workflow` column to sessions.
///
/// Marks sessions created by workflow LLM nodes. Enables filtering
/// workflow sessions in the sidebar and elsewhere.
/// Default is `false` - regular chat sessions are not workflow sessions.
fn migrate_v11(conn: &mut rusqlite::Connection) -> Result<(), Report<SessionStoreError>> {
    conn.execute_batch(
        "ALTER TABLE sessions ADD COLUMN is_workflow BOOLEAN NOT NULL DEFAULT FALSE",
    )
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
fn migrate_v12(conn: &mut rusqlite::Connection) -> Result<(), Report<SessionStoreError>> {
    conn.execute_batch(
        "ALTER TABLE session_history ADD COLUMN context_override TEXT NOT NULL DEFAULT 'default'",
    )
    .change_context(SessionStoreError)
    .attach("v12: add context_override column to session_history")?;
    conn.execute_batch(
        "UPDATE session_history SET context_override = 'forced_exclude' WHERE ignored = 1",
    )
    .change_context(SessionStoreError)
    .attach("v12: migrate ignored values to context_override")?;
    Ok(())
}

/// v13: Add `judge_meta` column to sessions.
///
/// Stores judge metadata as a nullable JSON text blob.
/// When NULL, the session is not a judge session.
fn migrate_v13(conn: &mut rusqlite::Connection) -> Result<(), Report<SessionStoreError>> {
    conn.execute_batch("ALTER TABLE sessions ADD COLUMN judge_meta TEXT")
        .change_context(SessionStoreError)
        .attach("v13: add judge_meta column to sessions")?;
    Ok(())
}

/// v14: Add `context_history` column to entries.
///
/// Stores the audit trail of context inclusion/exclusion changes as a JSON array
/// of `ContextChangeEvent`. Defaults to `'[]'` (empty audit) for existing rows.
fn migrate_v14(conn: &mut rusqlite::Connection) -> Result<(), Report<SessionStoreError>> {
    conn.execute_batch("ALTER TABLE entries ADD COLUMN context_history TEXT NOT NULL DEFAULT '[]'")
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
fn migrate_v15(conn: &mut rusqlite::Connection) -> Result<(), Report<SessionStoreError>> {
    conn.execute_batch(
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
    .change_context(SessionStoreError)
    .attach("v15: create sessions_new without strategy_state")?;

    conn.execute_batch(
        "INSERT INTO sessions_new (\
         id, title, updated_at, profile, blobs, parent_session, cwd, created_at,\
         archived, lifecycle_name, lifecycle_args, lifecycle_script_state, metadata,\
         is_workflow, judge_meta) \
         SELECT \
         id, title, updated_at, profile, blobs, parent_session, cwd, created_at,\
         archived, lifecycle_name, lifecycle_args, lifecycle_script_state, metadata,\
         is_workflow, judge_meta FROM sessions",
    )
    .change_context(SessionStoreError)
    .attach("v15: copy sessions into sessions_new")?;

    conn.execute_batch("DROP TABLE sessions")
        .change_context(SessionStoreError)
        .attach("v15: drop old sessions table")?;

    conn.execute_batch("ALTER TABLE sessions_new RENAME TO sessions")
        .change_context(SessionStoreError)
        .attach("v15: rename sessions_new to sessions")?;

    Ok(())
}

fn migrate_v16(conn: &mut rusqlite::Connection) -> Result<(), Report<SessionStoreError>> {
    conn.execute_batch(
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
    .change_context(SessionStoreError)
    .attach("v16: create sessions_new with is_automated + persist")?;

    conn.execute_batch(
        "INSERT INTO sessions_new (\
         id, title, updated_at, profile, blobs, parent_session, cwd, created_at,\
         archived, lifecycle_name, lifecycle_args, lifecycle_script_state, metadata,\
         is_automated, persist, judge_meta) \
         SELECT \
         id, title, updated_at, profile, blobs, parent_session, cwd, created_at,\
         archived, lifecycle_name, lifecycle_args, lifecycle_script_state, metadata,\
         is_workflow, TRUE, judge_meta FROM sessions",
    )
    .change_context(SessionStoreError)
    .attach("v16: copy sessions (is_workflow→is_automated, persist=TRUE)")?;

    conn.execute_batch("DROP TABLE sessions")
        .change_context(SessionStoreError)
        .attach("v16: drop old sessions table")?;

    conn.execute_batch("ALTER TABLE sessions_new RENAME TO sessions")
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
fn migrate_v17(conn: &mut rusqlite::Connection) -> Result<(), Report<SessionStoreError>> {
    let rows = query_map_rows(
        conn,
        "SELECT rowid, profile FROM sessions",
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        "v17",
    )?;

    for (rowid, profile) in rows {
        let new_profile = rewrite_profile_model(&profile);

        conn.execute(
            "UPDATE sessions SET profile = ? WHERE rowid = ?",
            params![new_profile, rowid],
        )
        .change_context(SessionStoreError)
        .attach("v17: update session profile")?;
    }

    // Idempotent: ignore "duplicate column" error if migration runs twice.
    match conn.execute("ALTER TABLE token_ledger ADD COLUMN model_used TEXT", []) {
        Ok(_) => {}
        Err(rusqlite::Error::SqliteFailure(_, Some(msg)))
            if msg.contains("duplicate column name") => {}
        Err(e) => {
            return Err(e)
                .change_context(SessionStoreError)
                .attach("v17: add model_used column to token_ledger");
        }
    }

    Ok(())
}

/// v18: Rename `entries.timestamp` column to `timing`.
///
/// The column now stores `EntryTiming` JSON (instant or streamed timing data)
/// rather than a plain timestamp string. The rename aligns the column name
/// with the Rust field it maps to.
fn migrate_v18(conn: &mut rusqlite::Connection) -> Result<(), Report<SessionStoreError>> {
    conn.execute_batch("ALTER TABLE entries RENAME COLUMN timestamp TO timing")
        .change_context(SessionStoreError)
        .attach("v18: rename entries.timestamp to timing")?;
    Ok(())
}

/// v19: Rewrite a bare-string `profile.model` inside the `metadata` blob.
///
/// Migration v17 rewrote the `profile` column from bare-string model to
/// `{"single": ...}`, but sessions written at v8+ carry their profile
/// embedded in the `metadata` blob (`PersistableCore`). The load path
/// treats the blob as authoritative - so a 0.65 blob (bare-string model)
/// fails to deserialize into `ModelSelection` and the session silently
/// drops out of the sidebar. This migration reaches into each blob's
/// embedded `profile` sub-object and applies the same rewrite v17 applied
/// to the column. Rows with `NULL` metadata (pre-v8 sessions) are skipped;
/// their column data was already fixed by v17.
fn migrate_v19(conn: &mut rusqlite::Connection) -> Result<(), Report<SessionStoreError>> {
    let rows = query_map_rows(
        conn,
        "SELECT rowid, metadata FROM sessions WHERE metadata IS NOT NULL",
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        "v19",
    )?;

    for (rowid, metadata) in rows {
        let new_metadata = rewrite_metadata_blob_model(&metadata);

        // Only UPDATE rows where the blob actually changed.
        if new_metadata == metadata {
            continue;
        }

        conn.execute(
            "UPDATE sessions SET metadata = ? WHERE rowid = ?",
            params![new_metadata, rowid],
        )
        .change_context(SessionStoreError)
        .attach("v19: update session metadata blob")?;
    }

    Ok(())
}

/// v20: Retire the six "zombie" `sessions` columns.
///
/// Columns `profile, blobs, cwd, lifecycle_name, lifecycle_args,
/// lifecycle_script_state` are never written by the app (writes go through
/// the `metadata` blob via `PersistableCore`). They survive only as a legacy
/// load fallback for pre-v8 rows whose `metadata` is `NULL`. This migration
/// backfills `metadata` for every NULL row by reconstructing the
/// [`PersistableCore`] from the columns, then rebuilds `sessions` without
/// them. After v20, every row has a metadata blob and the columns are gone.
///
/// `judge_meta` (vestigial — referenced only by an orphaned doc comment) is
/// dropped in the same rebuild.
fn migrate_v20(conn: &mut rusqlite::Connection) -> Result<(), Report<SessionStoreError>> {
    backfill_missing_metadata(conn)?;
    rebuild_sessions_without_zombies(conn)?;
    Ok(())
}

/// Reconstructs a `metadata` blob for every row where it is `NULL`.
///
/// Mirrors the pre-v20 legacy load branch (`sqlite.rs` `else { … }`):
/// deserialize `profile`/`blobs`/`lifecycle_args`/`lifecycle_script_state`
/// from their column JSON, parse `updated_at`/`created_at`/`parent_session`,
/// and serialize a full [`PersistableCore`].
fn backfill_missing_metadata(
    conn: &mut rusqlite::Connection,
) -> Result<(), Report<SessionStoreError>> {
    let rows = query_map_rows(
        conn,
        "SELECT rowid, id, title, updated_at, created_at, parent_session, \
                profile, blobs, cwd, lifecycle_name, lifecycle_args, \
                lifecycle_script_state \
         FROM sessions WHERE metadata IS NULL",
        LegacyRow::from_row,
        "v20-backfill",
    )?;

    for row in rows {
        let rowid = row.rowid;
        let blob = row.metadata_blob()?;
        conn.execute(
            "UPDATE sessions SET metadata = ? WHERE rowid = ?",
            params![blob, rowid],
        )
        .change_context(SessionStoreError)
        .attach("v20: backfill metadata")?;
    }

    Ok(())
}

/// 12-step SQLite table rebuild dropping the zombie + `judge_meta` columns.
///
/// `sessions` has no indexes beyond its implicit PK, so none need recreating.
fn rebuild_sessions_without_zombies(
    conn: &mut rusqlite::Connection,
) -> Result<(), Report<SessionStoreError>> {
    conn.execute_batch(
        "CREATE TABLE sessions_new (\
         id TEXT PRIMARY KEY,\
         title TEXT,\
         updated_at TEXT NOT NULL,\
         created_at TEXT NOT NULL,\
         parent_session TEXT DEFAULT NULL,\
         archived BOOLEAN NOT NULL DEFAULT FALSE,\
         metadata TEXT,\
         is_automated BOOLEAN NOT NULL DEFAULT FALSE,\
         persist BOOLEAN NOT NULL DEFAULT TRUE)",
    )
    .change_context(SessionStoreError)
    .attach("v20: create sessions_new without zombie columns")?;

    conn.execute_batch(
        "INSERT INTO sessions_new (\
         id, title, updated_at, created_at, parent_session, archived, metadata,\
         is_automated, persist) \
         SELECT id, title, updated_at, created_at, parent_session, archived, metadata,\
         is_automated, persist FROM sessions",
    )
    .change_context(SessionStoreError)
    .attach("v20: copy sessions into sessions_new")?;

    conn.execute_batch("DROP TABLE sessions")
        .change_context(SessionStoreError)
        .attach("v20: drop old sessions table")?;

    conn.execute_batch("ALTER TABLE sessions_new RENAME TO sessions")
        .change_context(SessionStoreError)
        .attach("v20: rename sessions_new to sessions")?;

    Ok(())
}

/// A legacy `sessions` row whose `metadata` is NULL — the pre-v8 shape that
/// the backfill reconstructs a [`PersistableCore`] blob from.
struct LegacyRow {
    /// SQLite rowid — the stable key for the subsequent UPDATE.
    rowid: i64,
    /// The pre-v8 column values, verbatim.
    columns: super::sqlite::LegacySessionColumns,
}

impl LegacyRow {
    fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Self {
            rowid: row.get(0)?,
            columns: super::sqlite::LegacySessionColumns {
                session_id: row.get(1)?,
                title: row.get(2)?,
                updated_at: row.get(3)?,
                created_at: row.get(4)?,
                parent_session: row.get(5)?,
                profile: row.get(6)?,
                blobs: row.get(7)?,
                cwd: row.get(8)?,
                lifecycle_name: row.get(9)?,
                lifecycle_args: row.get(10)?,
                lifecycle_script_state: row.get(11)?,
            },
        })
    }

    /// Reconstructs the [`PersistableCore`] blob from the legacy columns,
    /// exactly mirroring the pre-v20 legacy load branch in `sqlite.rs`.
    ///
    /// # Errors
    ///
    /// Returns an error if any column JSON fails to deserialize or the
    /// reconstructed blob cannot be serialized.
    fn metadata_blob(&self) -> Result<String, Report<SessionStoreError>> {
        super::sqlite::PersistableCore::blob_from_legacy_columns(&self.columns)
    }
}

/// Runs a `SELECT` and maps each row into `T` via `from_row`, releasing the
/// Runs `sql`, mapping each row via `map`. The statement borrow is released
/// before this returns, so the caller is free to mutate the connection.
///
/// Centralizes the prepare → query_map → collect pattern used by every
/// row-by-row JSON-surgery migration. The `tag` is attached to every error
/// for context.
fn query_map_rows<T, F>(
    conn: &mut rusqlite::Connection,
    sql: &str,
    map: F,
    tag: &str,
) -> Result<Vec<T>, Report<SessionStoreError>>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    let mut stmt = conn
        .prepare(sql)
        .change_context(SessionStoreError)
        .attach(format!("{tag}: prepare statement"))?;
    let mapped = stmt
        .query_map([], map)
        .change_context(SessionStoreError)
        .attach(format!("{tag}: map rows"))?;
    let mut rows = Vec::new();
    for row in mapped {
        rows.push(
            row.change_context(SessionStoreError)
                .attach(format!("{tag}: collect row"))?,
        );
    }
    Ok(rows)
}

/// Test-only: opens a pool at `db_path`, applies migrations up to (and including)
/// `target`, then runs `seed` against a held connection (FK=OFF, matching
/// `run_migrations`). Lets external test modules seed a DB at a specific legacy
/// schema version before letting the store re-open and migrate forward.
#[cfg(test)]
pub(crate) async fn seed_at_version<F>(db_path: &str, target: i32, seed: F)
where
    F: FnOnce(&mut rusqlite::Connection) -> rusqlite::Result<()> + Send + 'static,
{
    let pool = Pool::open(db_path).expect("open seed pool");
    pool.with_conn(move |conn| {
        bootstrap_tracking_table(conn).expect("bootstrap");
        apply_migrations_inner(conn, target);
        seed(conn).map_err(dao::Error::from)?;
        Ok(())
    })
    .await
    .expect("seed_at_version");
    // Drop the pool handle so the only reference is via the store's later open.
    drop(pool);
}

/// Applies migrations v0..=target on a held connection, recording each version.
///
/// This is the same dispatch `run_pending_migrations` uses, but stops at `target`
/// instead of running to completion. Used by `seed_at_version` and the test module's
/// `apply_migrations_up_to` to stand up a DB at a specific schema version.
#[cfg(test)]
#[expect(clippy::too_many_lines, reason = "one branch per migration")]
fn apply_migrations_inner(conn: &mut rusqlite::Connection, target: i32) {
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
        record_version(conn, 9, "rename_session_entries_to_session_history").expect("record v9");
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
    if target >= 18 {
        migrate_v18(conn).expect("v18");
        record_version(conn, 18, "rename_entries_timestamp_to_timing").expect("record v18");
    }
    if target >= 19 {
        migrate_v19(conn).expect("v19");
        record_version(conn, 19, "rewrite_metadata_blob_profile_model").expect("record v19");
    }
    if target >= 20 {
        migrate_v20(conn).expect("v20");
        record_version(conn, 20, "drop_zombie_columns_backfill_metadata").expect("record v20");
    }
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
    use crate::feat::session::session_store::sqlite::PersistableCore;
    use tempfile::TempDir;

    async fn make_pool() -> Pool {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("test.db");
        // Leak the temp dir so it outlives the test — migration tests don't
        // need cleanup, and dao::Pool holds the path by value.
        std::mem::forget(dir);
        Pool::open(path.to_string_lossy().to_string().as_str()).expect("open pool")
    }

    /// Synchronously applies migrations up to (and including) `target`.
    ///
    /// Runs inside a `with_conn` closure with FK=OFF (matching `run_migrations`).
    async fn apply_migrations_up_to(target: i32) -> Pool {
        let pool = make_pool().await;
        pool.with_conn(move |conn| {
            bootstrap_tracking_table(conn).expect("bootstrap");
            apply_migrations_inner(conn, target);
            Ok(())
        })
        .await
        .expect("apply_migrations_up_to");
        pool
    }

    #[tokio::test]
    async fn run_migrations_creates_tracking_table() {
        // Given a fresh database.
        let pool = make_pool().await;

        // When running migrations.
        run_migrations(&pool).await.unwrap();

        // Then the _migrations table has 21 entries.
        let rows: Vec<(i32, String)> = pool
            .with_conn(|conn| {
                let mut stmt =
                    conn.prepare("SELECT version, name FROM _migrations ORDER BY version")?;
                let mapped = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                let mut out = Vec::new();
                for row in mapped {
                    out.push(row?);
                }
                Ok(out)
            })
            .await
            .unwrap();

        assert_eq!(rows.len(), 21);
        assert_eq!(rows[0].0, 0);
        assert_eq!(rows[0].1, "create_initial_schema");
        assert_eq!(rows[1].0, 1);
        assert_eq!(rows[1].1, "add_cwd_column");
        assert_eq!(rows[2].0, 2);
        assert_eq!(rows[2].1, "add_created_at_column");
        assert_eq!(rows[3].0, 3);
        assert_eq!(rows[3].1, "add_ignored_to_session_entries");
        assert_eq!(rows[4].0, 4);
        assert_eq!(rows[4].1, "add_cost_to_token_ledger");
        assert_eq!(rows[5].0, 5);
        assert_eq!(rows[5].1, "add_lifecycle_columns_to_sessions");
        assert_eq!(rows[6].0, 6);
        assert_eq!(rows[6].1, "add_archived_column");
        assert_eq!(rows[7].0, 7);
        assert_eq!(rows[7].1, "add_lifecycle_script_state_column");
        assert_eq!(rows[8].0, 8);
        assert_eq!(rows[8].1, "add_metadata_column");
        assert_eq!(rows[9].0, 9);
        assert_eq!(rows[9].1, "rename_session_entries_to_session_history");
        assert_eq!(rows[10].0, 10);
        assert_eq!(rows[10].1, "consolidate_to_compaction_strategy");
        assert_eq!(rows[11].0, 11);
        assert_eq!(rows[11].1, "add_is_workflow_column");
        assert_eq!(rows[12].0, 12);
        assert_eq!(rows[12].1, "replace_ignored_with_context_override");
        assert_eq!(rows[13].0, 13);
        assert_eq!(rows[13].1, "add_judge_meta_column");
        assert_eq!(rows[14].0, 14);
        assert_eq!(rows[14].1, "add_context_history");
        assert_eq!(rows[19].0, 19);
        assert_eq!(rows[19].1, "rewrite_metadata_blob_profile_model");
        assert_eq!(rows[20].0, 20);
        assert_eq!(rows[20].1, "drop_zombie_columns_backfill_metadata");
    }

    #[tokio::test]
    async fn re_running_migrations_is_noop() {
        // Given a database with migrations already applied.
        let pool = make_pool().await;
        run_migrations(&pool).await.unwrap();

        // When running migrations again.
        run_migrations(&pool).await.unwrap();

        // Then no duplicate entries are added.
        let count: i64 = pool
            .with_conn(|conn| {
                conn.query_row("SELECT COUNT(*) AS count FROM _migrations", [], |r| {
                    r.get(0)
                })
                .map_err(dao::Error::from)
            })
            .await
            .unwrap();
        assert_eq!(count, 21);
    }

    /// Verifies that each migration guard uses `<` not `<=`.
    ///
    /// For each version N (0..=19), we build a database at exactly version N
    /// by calling individual migration functions, then re-run `run_migrations`.
    /// It must succeed (applying only v(N+1) through v20) and produce exactly
    /// 21 migration rows.
    ///
    /// If `current < N` were mutated to `current <= N`, vN would re-run when
    /// current == N. Most migrations would fail (duplicate table/column),
    /// and those that don't fail would produce a duplicate _migrations row,
    /// causing the count assertion to fail.
    #[tokio::test]
    async fn migration_guards_do_not_reapply_completed_version() {
        for target_version in 0..=19_i32 {
            let pool = apply_migrations_up_to(target_version).await;

            // Re-running should succeed - applying only versions > target_version.
            run_migrations(&pool).await.unwrap_or_else(|e| {
                panic!("re-run at target_version={target_version} should succeed: {e:?}")
            });

            // Verify no duplicate rows: exactly 21 migration rows total.
            let count: i64 = pool
                .with_conn(|conn| {
                    conn.query_row("SELECT COUNT(*) AS count FROM _migrations", [], |r| {
                        r.get(0)
                    })
                    .map_err(dao::Error::from)
                })
                .await
                .unwrap();
            assert_eq!(
                count, 21,
                "at target_version={target_version}: expected 21 migration rows, no duplicates"
            );
        }
    }

    #[tokio::test]
    async fn fresh_database_has_all_tables() {
        // Given a fresh database.
        let pool = make_pool().await;

        // When running migrations.
        run_migrations(&pool).await.unwrap();

        // Then all expected tables exist.
        let tables: Vec<String> = pool
            .with_conn(|conn| {
                let mut stmt = conn
                    .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")?;
                let mapped = stmt.query_map([], |r| r.get::<_, String>(0))?;
                let mut out = Vec::new();
                for row in mapped {
                    out.push(row?);
                }
                Ok(out)
            })
            .await
            .unwrap();
        let names: Vec<&str> = tables.iter().map(String::as_str).collect();
        assert!(names.contains(&"_migrations"));
        assert!(names.contains(&"entries"));
        assert!(names.contains(&"session_history"));
        assert!(names.contains(&"sessions"));
        assert!(names.contains(&"token_ledger"));
    }

    #[tokio::test]
    async fn migrate_v10_consolidates_strategy_state() {
        // Given a database with sessions containing mixed strategy state and profile.
        let pool = apply_migrations_up_to(9).await;
        pool.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, title, updated_at, created_at, cwd, profile, strategy_state, blobs, lifecycle_script_state) \
                 VALUES ('test-1', 'Test', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z', '.', \
                 '{\"strategy\": \"sliding_window\", \"model\": \"test\", \"persona_name\": \"coding-assistant\", \"token_budget\": 150000, \"sliding_window_size\": 5}', \
                 '{\"passthrough\": \"Passthrough\", \"compaction\": {\"compaction\": {\"compaction_count\": 2}}}', \
                 '{}', 'nothing_ran')",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        // When running migration v10.
        pool.with_conn(|conn| Ok(migrate_v10(conn).expect("migrate v10")))
            .await
            .unwrap();

        // Then strategy_state only has the compaction key.
        let (state_str, profile_str): (String, String) = pool
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT strategy_state, profile FROM sessions WHERE id = 'test-1'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(dao::Error::from)
            })
            .await
            .unwrap();

        let state_map: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&state_str).expect("parse state");
        assert!(state_map.contains_key("compaction"));
        assert!(!state_map.contains_key("passthrough"));
        assert!(!state_map.contains_key("sliding_window"));
        assert!(!state_map.contains_key("token_budget"));

        // And profile.strategy is "compaction".
        let profile: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&profile_str).expect("parse profile");
        assert_eq!(profile["strategy"], "compaction");
    }

    #[tokio::test]
    async fn migrate_v10_inserts_default_when_no_compaction_key() {
        // Given a database with a session having no compaction key in strategy_state.
        let pool = apply_migrations_up_to(9).await;
        pool.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, title, updated_at, created_at, cwd, profile, strategy_state, blobs, lifecycle_script_state) \
                 VALUES ('test-2', 'Test', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z', '.', \
                 '{\"strategy\": \"passthrough\", \"model\": \"test\"}', \
                 '{\"passthrough\": \"Passthrough\"}', \
                 '{}', 'nothing_ran')",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        // When running migration v10.
        pool.with_conn(|conn| Ok(migrate_v10(conn).expect("migrate v10")))
            .await
            .unwrap();

        // Then strategy_state has a compaction key with default data.
        let state_str: String = pool
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT strategy_state FROM sessions WHERE id = 'test-2'",
                    [],
                    |r| r.get(0),
                )
                .map_err(dao::Error::from)
            })
            .await
            .unwrap();
        let state_map: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&state_str).expect("parse state");
        assert!(state_map.contains_key("compaction"));
        assert!(!state_map.contains_key("passthrough"));
    }

    #[tokio::test]
    async fn migrate_v15_drops_strategy_state_column() {
        // Given a database built to v14 (strategy_state column still present) with a session in it.
        let pool = apply_migrations_up_to(14).await;
        pool.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, title, updated_at, created_at, cwd, profile, strategy_state, blobs, lifecycle_script_state) \
                 VALUES ('keep-me', 'Survivor', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z', '.', \
                 '{}', '{}', '{}', 'nothing_ran')",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        // When running migration v15.
        pool.with_conn(|conn| Ok(migrate_v15(conn).expect("migrate v15")))
            .await
            .unwrap();

        // Then the sessions table no longer has a strategy_state column.
        let cols: Vec<String> = column_names(&pool, "sessions").await;
        assert!(
            !cols.iter().any(|c| c == "strategy_state"),
            "strategy_state column should be dropped; found columns: {cols:?}"
        );

        // And the session row survived the table rebuild.
        let ids: Vec<String> = pool
            .with_conn(|conn| {
                let mut stmt = conn.prepare("SELECT id FROM sessions")?;
                let mapped = stmt.query_map([], |r| r.get::<_, String>(0))?;
                let mut out = Vec::new();
                for row in mapped {
                    out.push(row?);
                }
                Ok(out)
            })
            .await
            .unwrap();
        assert!(
            ids.iter().any(|i| i == "keep-me"),
            "session 'keep-me' must survive the v15 table rebuild"
        );
    }

    #[tokio::test]
    async fn v15_rebuild_preserves_session_history_rows() {
        // Given a v14 database with a session linked to an entry via session_history,
        // and the connection running with foreign_keys=ON — as the production
        // bootstrap connection does.
        let pool = apply_migrations_up_to(14).await;
        pool.with_conn(|conn| {
            conn.pragma_update(None, "foreign_keys", "ON")?;
            seed_session_with_children(conn);
            Ok(())
        })
        .await
        .unwrap();

        // When running migrations (applies v15, the DROP TABLE sessions rebuild).
        run_migrations(&pool).await.unwrap();

        // Then the session_history junction row survived the rebuild.
        let count: i64 = table_count(&pool, "session_history").await;
        assert_eq!(
            count, 1,
            "session_history junction row must survive v15 rebuild"
        );
    }

    #[tokio::test]
    async fn v15_rebuild_preserves_token_ledger_rows() {
        // Given a v14 database with a token_ledger row for a session,
        // and the connection running with foreign_keys=ON.
        let pool = apply_migrations_up_to(14).await;
        pool.with_conn(|conn| {
            conn.pragma_update(None, "foreign_keys", "ON")?;
            seed_session_with_children(conn);
            Ok(())
        })
        .await
        .unwrap();

        // When running migrations (applies v15).
        run_migrations(&pool).await.unwrap();

        // Then the token_ledger row survived the rebuild.
        let count: i64 = table_count(&pool, "token_ledger").await;
        assert_eq!(count, 1, "token_ledger row must survive v15 rebuild");
    }

    /// Seeds a v14 database with one session, one entry, one session_history
    /// junction row linking them, and one token_ledger row.
    ///
    /// Call after `apply_migrations_up_to(14)`.
    fn seed_session_with_children(conn: &mut rusqlite::Connection) {
        conn.execute(
            "INSERT INTO sessions (id, title, updated_at, created_at, cwd, profile, strategy_state, blobs, lifecycle_script_state) \
             VALUES ('s-1', 'T', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z', '.', '{}', '{}', '{}', 'nothing_ran')",
            [],
        )
        .expect("seed session");
        conn.execute(
            "INSERT INTO entries (id, timestamp, kind) VALUES ('e-1', '2024-01-01T00:00:00Z', '\"User\"')",
            [],
        )
        .expect("seed entry");
        conn.execute(
            "INSERT INTO session_history (session_id, entry_id, ordinal) VALUES ('s-1', 'e-1', 0)",
            [],
        )
        .expect("seed junction");
        conn.execute(
            "INSERT INTO token_ledger (session_id, timestamp, tokens_sent, tokens_received) \
             VALUES ('s-1', '2024-01-01T00:00:00Z', 10, 20)",
            [],
        )
        .expect("seed ledger");
    }

    #[tokio::test]
    async fn run_migrations_preserves_child_rows_and_leaves_fk_clean() {
        // Given a v14 database with FK=ON and a full session (junction + ledger).
        let pool = apply_migrations_up_to(14).await;
        pool.with_conn(|conn| {
            conn.pragma_update(None, "foreign_keys", "ON")?;
            seed_session_with_children(conn);
            Ok(())
        })
        .await
        .unwrap();

        // When running the full migration suite.
        run_migrations(&pool).await.unwrap();

        // Then both child tables retain their row.
        assert_eq!(
            table_count(&pool, "session_history").await,
            1,
            "session_history row preserved"
        );
        assert_eq!(
            table_count(&pool, "token_ledger").await,
            1,
            "token_ledger row preserved"
        );

        // And foreign_key_check reports no violations on the final state.
        let violations: Vec<String> = pool
            .with_conn(|conn| {
                let mut stmt = conn.prepare("PRAGMA foreign_key_check")?;
                let mapped = stmt.query_map([], |r| r.get::<_, String>(0))?;
                let mut out = Vec::new();
                for row in mapped {
                    out.push(row?);
                }
                Ok(out)
            })
            .await
            .unwrap();
        assert!(
            violations.is_empty(),
            "foreign_key_check must be clean after migrations; found: {violations:?}"
        );
    }

    #[tokio::test]
    async fn migrate_v17_rewrites_bare_string_model_to_single() {
        // Given a database at v16 with a session having a bare-string model.
        let pool = apply_migrations_up_to(16).await;
        pool.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, title, updated_at, created_at, cwd, profile, blobs, lifecycle_script_state, is_automated, persist) \
                 VALUES ('test-v17', 'Test', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z', '.', \
                 '{\"strategy\": \"sliding_window\", \"model\": \"ollama/llama3\", \"persona_name\": \"coding-assistant\", \"token_budget\": 150000, \"sliding_window_size\": 5}', \
                 '{}', 'nothing_ran', 0, 0)",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        // When running migration v17.
        pool.with_conn(|conn| Ok(migrate_v17(conn).expect("migrate v17")))
            .await
            .unwrap();

        // Then the profile JSON has model rewritten as {"single":"ollama/llama3"}.
        let profile_str: String = pool
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT profile FROM sessions WHERE id = 'test-v17'",
                    [],
                    |r| r.get(0),
                )
                .map_err(dao::Error::from)
            })
            .await
            .unwrap();
        let profile: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&profile_str).expect("parse profile");
        assert_eq!(
            profile["model"],
            serde_json::json!({"single": "ollama/llama3"})
        );
        // And other profile fields are preserved.
        assert_eq!(profile["strategy"], "sliding_window");
        assert_eq!(profile["persona_name"], "coding-assistant");
    }

    #[tokio::test]
    async fn migrate_v17_handles_no_provider_id() {
        // Given a database at v16 with a session having <none> as model.
        let pool = apply_migrations_up_to(16).await;
        pool.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, title, updated_at, created_at, cwd, profile, blobs, lifecycle_script_state, is_automated, persist) \
                 VALUES ('test-none', 'Test', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z', '.', \
                 '{\"strategy\": \"sliding_window\", \"model\": \"<none>\", \"persona_name\": \"coding-assistant\"}', \
                 '{}', 'nothing_ran', 0, 0)",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        // When running migration v17.
        pool.with_conn(|conn| Ok(migrate_v17(conn).expect("migrate v17")))
            .await
            .unwrap();

        // Then the profile JSON has model rewritten as {"single":"<none>"}.
        let profile_str: String = pool
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT profile FROM sessions WHERE id = 'test-none'",
                    [],
                    |r| r.get(0),
                )
                .map_err(dao::Error::from)
            })
            .await
            .unwrap();
        let profile: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&profile_str).expect("parse profile");
        assert_eq!(profile["model"], serde_json::json!({"single": "<none>"}));
    }

    #[tokio::test]
    async fn migrate_v17_is_idempotent() {
        // Given a database at v16 with a session.
        let pool = apply_migrations_up_to(16).await;
        pool.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, title, updated_at, created_at, cwd, profile, blobs, lifecycle_script_state, is_automated, persist) \
                 VALUES ('test-idem', 'Test', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z', '.', \
                 '{\"strategy\": \"sliding_window\", \"model\": \"ollama/llama3\"}', \
                 '{}', 'nothing_ran', 0, 0)",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        // When running migration v17 twice.
        pool.with_conn(|conn| Ok(migrate_v17(conn).expect("first")))
            .await
            .unwrap();
        pool.with_conn(|conn| Ok(migrate_v17(conn).expect("second")))
            .await
            .unwrap();

        // Then the profile is still correct, not double-wrapped.
        let profile_str: String = pool
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT profile FROM sessions WHERE id = 'test-idem'",
                    [],
                    |r| r.get(0),
                )
                .map_err(dao::Error::from)
            })
            .await
            .unwrap();
        let profile: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&profile_str).expect("parse profile");
        assert_eq!(
            profile["model"],
            serde_json::json!({"single": "ollama/llama3"})
        );
    }

    #[tokio::test]
    async fn migrate_v17_adds_model_used_column() {
        // Given a database at v16 (no model_used column).
        let pool = apply_migrations_up_to(16).await;

        // When running migration v17.
        pool.with_conn(|conn| Ok(migrate_v17(conn).expect("migrate v17")))
            .await
            .unwrap();

        // Then the token_ledger table has a model_used column.
        let cols = column_names(&pool, "token_ledger").await;
        assert!(
            cols.contains(&"model_used".to_string()),
            "expected model_used column, got: {cols:?}"
        );
    }

    #[tokio::test]
    async fn migrate_v18_renames_timestamp_and_preserves_value() {
        // Given a database at v16 (timestamp column, before rename).
        let pool = apply_migrations_up_to(15).await;
        pool.with_conn(|conn| {
            migrate_v16(conn).expect("v16");
            record_version(conn, 16, "add_persist_and_is_automated").expect("record v16");
            Ok(())
        })
        .await
        .unwrap();

        // Insert a row with the old `timestamp` column.
        let legacy_ts = "2024-01-15T10:30:00Z";
        pool.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO entries (id, timestamp, kind) VALUES ('e1', ?, 'user')",
                params![legacy_ts],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        // When running migrations (v18 renames timestamp → timing).
        run_migrations(&pool).await.unwrap();

        // Then the column is renamed and the value is preserved.
        let timing: String = pool
            .with_conn(|conn| {
                conn.query_row("SELECT timing FROM entries WHERE id = 'e1'", [], |r| {
                    r.get(0)
                })
                .map_err(dao::Error::from)
            })
            .await
            .unwrap();
        assert_eq!(timing, legacy_ts, "value preserved through rename");
    }

    // ── v19: rewrite embedded profile.model in metadata blob ─────────
    //
    // A realistic 0.65 metadata blob looks like a PersistableCore JSON with
    // profile.model as a bare string. The helper constructs one.

    /// Builds a metadata blob (PersistableCore shape) whose profile.model is the
    /// given JSON value, so tests can inject bare-string, object, or other shapes.
    fn make_metadata_blob(model: &serde_json::Value) -> String {
        let blob = serde_json::json!({
            "session_id": "s1",
            "title": "T",
            "updated_at": "2024-01-01T00:00:00Z",
            "created_at": "2024-01-01T00:00:00Z",
            "profile": {
                "model": model.clone(),
                "persona_name": "coding-assistant"
            },
            "cwd": ".",
            "blobs": {},
            "lifecycle_args": [],
            "lifecycle_script_state": "nothing_ran"
        });
        serde_json::to_string(&blob).expect("serialize blob")
    }

    /// Inserts a session row with the given id and metadata blob at v18 schema.
    async fn insert_v19_session(pool: &Pool, id: &str, metadata: &str) {
        let id = id.to_owned();
        let metadata = metadata.to_owned();
        pool.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO sessions (id, title, updated_at, created_at, cwd, profile, blobs, lifecycle_script_state, is_automated, persist, metadata) VALUES (?, 'T', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z', '.', '{}', '{}', 'nothing_ran', 0, 0, ?)",
                params![id, metadata],
            )?;
            Ok(())
        })
        .await
        .expect("insert v19 test session");
    }

    /// Builds a v18 database (the pre-v19 state) so v19 tests can run in isolation.
    async fn make_v18_pool() -> Pool {
        let pool = apply_migrations_up_to(17).await;
        pool.with_conn(|conn| {
            migrate_v18(conn).expect("v18");
            record_version(conn, 18, "rename_entries_timestamp_to_timing").expect("record v18");
            Ok(())
        })
        .await
        .expect("build v18");
        pool
    }

    #[tokio::test]
    async fn migrate_v19_rewrites_bare_string_model_in_blob() {
        // Given a database at v18 with a 0.65-shape metadata blob (bare-string model).
        let pool = make_v18_pool().await;
        let blob = make_metadata_blob(&serde_json::json!("ollama/llama3"));
        insert_v19_session(&pool, "s1", &blob).await;

        // When running migration v19.
        pool.with_conn(|conn| Ok(migrate_v19(conn).expect("migrate v19")))
            .await
            .unwrap();

        // Then the blob's embedded profile.model is rewritten to {"single":...}.
        let meta_str: String = pool
            .with_conn(|conn| {
                conn.query_row("SELECT metadata FROM sessions WHERE id = 's1'", [], |r| {
                    r.get(0)
                })
                .map_err(dao::Error::from)
            })
            .await
            .unwrap();
        let meta: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&meta_str).expect("parse metadata");
        let profile = meta["profile"].as_object().expect("profile is object");
        assert_eq!(
            profile["model"],
            serde_json::json!({"single": "ollama/llama3"})
        );
        // And other profile fields are preserved.
        assert_eq!(profile["persona_name"], "coding-assistant");
    }

    #[tokio::test]
    async fn migrate_v19_leaves_already_object_model_unchanged() {
        // Given a database at v18 with a 0.66-shape blob (model already an object).
        let pool = make_v18_pool().await;
        let blob = make_metadata_blob(&serde_json::json!({"single": "ollama/llama3"}));
        insert_v19_session(&pool, "s1", &blob).await;

        // When running migration v19 twice (idempotency check).
        pool.with_conn(|conn| Ok(migrate_v19(conn).expect("first")))
            .await
            .unwrap();
        pool.with_conn(|conn| Ok(migrate_v19(conn).expect("second")))
            .await
            .unwrap();

        // Then the blob is unchanged - model is still {"single":...}, not double-wrapped.
        let meta_str: String = pool
            .with_conn(|conn| {
                conn.query_row("SELECT metadata FROM sessions WHERE id = 's1'", [], |r| {
                    r.get(0)
                })
                .map_err(dao::Error::from)
            })
            .await
            .unwrap();
        let meta: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&meta_str).expect("parse metadata");
        let profile = meta["profile"].as_object().expect("profile is object");
        assert_eq!(
            profile["model"],
            serde_json::json!({"single": "ollama/llama3"})
        );
    }

    #[tokio::test]
    async fn migrate_v19_leaves_blob_without_model_field_unchanged() {
        // Given a blob whose profile has no model field at all.
        let pool = make_v18_pool().await;
        let blob = "{\"session_id\":\"s1\",\"profile\":{\"persona_name\":\"x\"}}";
        insert_v19_session(&pool, "s1", blob).await;

        // When running migration v19.
        pool.with_conn(|conn| Ok(migrate_v19(conn).expect("migrate v19")))
            .await
            .unwrap();

        // Then the blob is semantically unchanged (no model added, no fields lost).
        let meta_str: String = pool
            .with_conn(|conn| {
                conn.query_row("SELECT metadata FROM sessions WHERE id = 's1'", [], |r| {
                    r.get(0)
                })
                .map_err(dao::Error::from)
            })
            .await
            .unwrap();
        let after: serde_json::Value = serde_json::from_str(&meta_str).expect("parse after");
        let before: serde_json::Value = serde_json::from_str(blob).expect("parse before");
        assert_eq!(
            after, before,
            "blob without model field must be semantically untouched"
        );
    }

    #[tokio::test]
    async fn migrate_v19_preserves_alloy_model() {
        // Given a blob with an Alloy model (object form, must not be re-wrapped as Single).
        let pool = make_v18_pool().await;
        let blob = make_metadata_blob(&serde_json::json!({
            "Alloy": {"models": ["a", "b"], "strategy": "RoundRobin"}
        }));
        insert_v19_session(&pool, "s1", &blob).await;

        // When running migration v19.
        pool.with_conn(|conn| Ok(migrate_v19(conn).expect("migrate v19")))
            .await
            .unwrap();

        // Then the Alloy model is preserved exactly, not wrapped as Single.
        let meta_str: String = pool
            .with_conn(|conn| {
                conn.query_row("SELECT metadata FROM sessions WHERE id = 's1'", [], |r| {
                    r.get(0)
                })
                .map_err(dao::Error::from)
            })
            .await
            .unwrap();
        let meta: serde_json::Value = serde_json::from_str(&meta_str).expect("parse");
        let model = &meta["profile"]["model"];
        assert_eq!(
            model,
            &serde_json::json!({"Alloy": {"models": ["a", "b"], "strategy": "RoundRobin"}}),
            "Alloy model must be preserved, not wrapped as Single"
        );
    }

    // ── v20: backfill NULL metadata, then drop zombie columns ─────────

    /// Builds a v19 database with one NULL-metadata (legacy pre-v8) row, ready
    /// for v20 testing. Mirrors the fixture the retired
    /// `migrate_v19_skips_null_metadata_rows` test used.
    async fn make_v19_pool_with_legacy_row() -> Pool {
        let pool = make_v18_pool().await;
        pool.with_conn(|conn| {
            migrate_v19(conn).expect("v19");
            record_version(conn, 19, "rewrite_metadata_blob_profile_model").expect("record v19");
            conn.execute(
                "INSERT INTO sessions (id, title, updated_at, created_at, cwd, profile, blobs, \
                 lifecycle_script_state, is_automated, persist) \
                 VALUES ('legacy', 'Legacy', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z', '.', \
                 '{\"model\":{\"single\":\"ollama/llama3\"},\"persona_name\":\"coding-assistant\"}', '{}', 'nothing_ran', 0, 0)",
                [],
            )?;
            Ok(())
        })
        .await
        .expect("seed legacy row");
        pool
    }

    #[tokio::test]
    async fn migrate_v20_backfills_null_metadata_and_drops_zombies() {
        // Given a v19 database with one NULL-metadata (pre-v8) row.
        let pool = make_v19_pool_with_legacy_row().await;

        // When migrating to v20.
        run_migrations(&pool).await.expect("migrate to v20");

        // Then sessions has exactly 9 columns.
        let cols = column_names(&pool, "sessions").await;
        assert_eq!(
            cols.len(),
            9,
            "sessions must have exactly 9 columns; got {cols:?}"
        );
        assert!(
            !cols.iter().any(|c| matches!(
                c.as_str(),
                "profile"
                    | "blobs"
                    | "cwd"
                    | "lifecycle_name"
                    | "lifecycle_args"
                    | "lifecycle_script_state"
                    | "judge_meta"
            )),
            "zombie + judge_meta columns must be dropped; got {cols:?}"
        );

        // And the legacy row's metadata was backfilled (non-NULL, parseable, with
        // the expected model shape).
        let meta_str: Option<String> = pool
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT metadata FROM sessions WHERE id = 'legacy'",
                    [],
                    |r| r.get(0),
                )
                .map_err(dao::Error::from)
            })
            .await
            .expect("query legacy metadata");
        let meta_str = meta_str.expect("metadata must be backfilled (non-NULL)");
        let meta: serde_json::Value =
            serde_json::from_str(&meta_str).expect("backfilled blob parses");
        // v17 already rewrote the column, so the backfilled blob carries the
        // {"single": ...} shape.
        assert_eq!(
            meta["profile"]["model"],
            serde_json::json!({"single": "ollama/llama3"}),
            "backfilled blob must carry the v17-rewritten model"
        );
    }

    #[tokio::test]
    async fn migrate_v20_backfilled_blob_loads_as_persistable_core() {
        // Given a v19 database with one NULL-metadata (pre-v8) row.
        let pool = make_v19_pool_with_legacy_row().await;

        // When migrating to v20.
        run_migrations(&pool).await.expect("migrate to v20");

        // Then the backfilled blob deserializes cleanly into PersistableCore.
        let meta_str: String = pool
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT metadata FROM sessions WHERE id = 'legacy'",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .map_err(dao::Error::from)
            })
            .await
            .expect("query backfilled blob");
        let meta: serde_json::Value =
            serde_json::from_str(&meta_str).expect("backfilled blob parses as JSON");
        assert_eq!(meta["title"].as_str(), Some("Legacy"));
    }

    // ── shared test helpers ───────────────────────────────────────────

    async fn table_count(pool: &Pool, table: &str) -> i64 {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        pool.with_conn(move |conn| {
            conn.query_row(&sql, [], |r| r.get(0))
                .map_err(dao::Error::from)
        })
        .await
        .expect("count")
    }

    async fn column_names(pool: &Pool, table: &str) -> Vec<String> {
        let table = table.to_owned();
        pool.with_conn(move |conn| {
            let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
            let mapped = stmt.query_map([], |r| r.get::<_, String>(1))?;
            let mut out = Vec::new();
            for row in mapped {
                out.push(row?);
            }
            Ok(out)
        })
        .await
        .expect("table_info")
    }
}
