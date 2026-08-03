//! The migration runner and individual migrations (v0..=v22).
//!
//! Ported verbatim from jinn-domain's `migrator.rs` so the schema crate is the
//! single source of truth. Three mechanical changes from the original:
//!
//! - error type `SessionStoreError` → [`crate::SchemaMigrationError`]
//! - `super::sqlite::LegacySessionColumns` → [`crate::legacy::LegacySessionColumns`]
//! - the v20 backfill builds the blob via [`crate::PersistableCoreV20`] (a
//!   version-pinned snapshot) instead of importing the live `PersistableCore`
//!
//! The DDL strings, version guards, and JSON surgery helpers are unchanged.

use std::collections::HashMap;

use error_stack::{Report, ResultExt as _};
use rusqlite::params;

use crate::legacy::LegacySessionColumns;
use crate::SchemaMigrationError;

/// Runs all pending migrations in order.
///
/// Bootstraps the tracking table, reads the current version, and runs every
/// unapplied migration sequentially. The caller ([`crate::run_migrations`]) is
/// responsible for the FK toggle around this call.
///
/// # Errors
///
/// Returns an error if any migration or version recording fails.
pub(crate) fn run_pending(
    conn: &mut rusqlite::Connection,
) -> Result<(), Report<SchemaMigrationError>> {
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
    if current < 21 {
        migrate_v21(conn)?;
        record_version(conn, 21, "add_discord_thread_table")?;
    }
    if current < 22 {
        migrate_v22(conn)?;
        record_version(conn, 22, "add_entry_blobs_table")?;
    }
    if current < 23 {
        migrate_v23(conn)?;
        record_version(conn, 23, "strip_s_prefix_from_session_ids")?;
    }
    if current < 24 {
        migrate_v24(conn)?;
        record_version(conn, 24, "add_token_ledger_prompt_cached_columns")?;
    }
    Ok(())
}

// ── Tracking table ───────────────────────────────────────────────────────

/// Creates the `_migrations` tracking table.
///
/// This is the only place `IF NOT EXISTS` is used - the tracking table must
/// bootstrap itself before version checking can begin. All other migrations
/// use strict DDL so failures are loud.
pub fn bootstrap_tracking_table(
    conn: &mut rusqlite::Connection,
) -> Result<(), Report<SchemaMigrationError>> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _migrations (\
         version INTEGER NOT NULL,\
         name TEXT NOT NULL,\
         applied_at TEXT NOT NULL DEFAULT (datetime('now')))",
    )
    .change_context(SchemaMigrationError)
    .attach("failed to create _migrations table")?;
    Ok(())
}

/// Reads the highest migration version from the tracking table.
///
/// Returns -1 if no migrations have been recorded (empty database).
fn current_version(conn: &mut rusqlite::Connection) -> Result<i32, Report<SchemaMigrationError>> {
    let version: Option<i32> = conn
        .query_row(
            "SELECT MAX(version) AS version FROM _migrations",
            [],
            |row| row.get(0),
        )
        .change_context(SchemaMigrationError)
        .attach("failed to query migration version")?;
    Ok(version.unwrap_or(-1))
}

/// Records a completed migration in the tracking table.
pub fn record_version(
    conn: &mut rusqlite::Connection,
    version: i32,
    name: &str,
) -> Result<(), Report<SchemaMigrationError>> {
    conn.execute(
        "INSERT INTO _migrations (version, name) VALUES (?, ?)",
        params![version, name],
    )
    .change_context(SchemaMigrationError)
    .attach(format!("failed to record migration v{version}"))?;
    Ok(())
}

// ── Migrations ───────────────────────────────────────────────────────────

/// v0: Initial schema - sessions, entries, session_entries, token_ledger.
pub fn migrate_v0(conn: &mut rusqlite::Connection) -> Result<(), Report<SchemaMigrationError>> {
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
    .change_context(SchemaMigrationError)
    .attach("v0: create sessions table")?;

    conn.execute_batch(
        "CREATE TABLE entries (\
         id TEXT PRIMARY KEY,\
         timestamp TEXT NOT NULL,\
         kind TEXT NOT NULL)",
    )
    .change_context(SchemaMigrationError)
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
    .change_context(SchemaMigrationError)
    .attach("v0: create session_entries table")?;

    conn.execute_batch(
        "CREATE TABLE token_ledger (\
         id INTEGER PRIMARY KEY AUTOINCREMENT,\
         session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,\
         timestamp TEXT NOT NULL,\
         tokens_sent INTEGER NOT NULL,\
         tokens_received INTEGER NOT NULL)",
    )
    .change_context(SchemaMigrationError)
    .attach("v0: create token_ledger table")?;

    conn.execute_batch(
        "CREATE INDEX idx_session_entries_session ON session_entries(session_id, ordinal)",
    )
    .change_context(SchemaMigrationError)
    .attach("v0: create session_entries index")?;

    conn.execute_batch("CREATE INDEX idx_token_ledger_session ON token_ledger(session_id)")
        .change_context(SchemaMigrationError)
        .attach("v0: create token_ledger index")?;
    Ok(())
}

/// v1: Add `cwd` column to sessions.
pub fn migrate_v1(conn: &mut rusqlite::Connection) -> Result<(), Report<SchemaMigrationError>> {
    conn.execute_batch("ALTER TABLE sessions ADD COLUMN cwd TEXT NOT NULL DEFAULT '.'")
        .change_context(SchemaMigrationError)
        .attach("v1: add cwd column to sessions")?;
    Ok(())
}

/// v2: Add `created_at` column to sessions.
pub fn migrate_v2(conn: &mut rusqlite::Connection) -> Result<(), Report<SchemaMigrationError>> {
    conn.execute_batch("ALTER TABLE sessions ADD COLUMN created_at TEXT NOT NULL DEFAULT ''")
        .change_context(SchemaMigrationError)
        .attach("v2: add created_at column to sessions")?;
    Ok(())
}

/// v3: Add `ignored` column to session_entries.
///
/// Compaction marks entries as ignored when they've been summarized.
/// Default is `false` (entry is active and visible during prompt assembly).
pub fn migrate_v3(conn: &mut rusqlite::Connection) -> Result<(), Report<SchemaMigrationError>> {
    conn.execute_batch(
        "ALTER TABLE session_entries ADD COLUMN ignored BOOLEAN NOT NULL DEFAULT FALSE",
    )
    .change_context(SchemaMigrationError)
    .attach("v3: add ignored column to session_entries")?;
    Ok(())
}

/// v4: Add `cost` column to token_ledger.
///
/// Tracks per-request cost in USD as reported by the provider (e.g. OpenRouter).
pub fn migrate_v4(conn: &mut rusqlite::Connection) -> Result<(), Report<SchemaMigrationError>> {
    conn.execute_batch("ALTER TABLE token_ledger ADD COLUMN cost DOUBLE")
        .change_context(SchemaMigrationError)
        .attach("v4: add cost column to token_ledger")?;
    Ok(())
}

/// v5: Add `lifecycle_name` and `lifecycle_args` columns to sessions.
///
/// `lifecycle_name` is NULL for sessions created without a lifecycle.
/// `lifecycle_args` is a JSON array of strings, defaulting to empty.
pub fn migrate_v5(conn: &mut rusqlite::Connection) -> Result<(), Report<SchemaMigrationError>> {
    conn.execute_batch("ALTER TABLE sessions ADD COLUMN lifecycle_name TEXT DEFAULT NULL")
        .change_context(SchemaMigrationError)
        .attach("v5: add lifecycle_name column to sessions")?;
    conn.execute_batch("ALTER TABLE sessions ADD COLUMN lifecycle_args TEXT NOT NULL DEFAULT '[]'")
        .change_context(SchemaMigrationError)
        .attach("v5: add lifecycle_args column to sessions")?;
    Ok(())
}

/// v6: Add `archived` column to sessions.
///
/// Sessions default to unarchived. Closing a session sets `archived = TRUE`.
/// On startup, only unarchived sessions are loaded into memory.
#[expect(clippy::expect_used, reason = "infallible")]
pub fn migrate_v6(conn: &mut rusqlite::Connection) {
    conn.execute_batch("ALTER TABLE sessions ADD COLUMN archived BOOLEAN NOT NULL DEFAULT FALSE")
        .expect("v6: add archived column to sessions");
}

/// v7: Add `lifecycle_script_state` column to sessions.
///
/// Persists the `LifecycleScriptState` enum so teardown runs correctly
/// after app restart for sessions that had setup run.
/// Default is `'nothing_ran'` - matching the enum's default.
pub fn migrate_v7(conn: &mut rusqlite::Connection) -> Result<(), Report<SchemaMigrationError>> {
    conn.execute_batch(
        "ALTER TABLE sessions ADD COLUMN lifecycle_script_state TEXT NOT NULL DEFAULT 'nothing_ran'",
    )
    .change_context(SchemaMigrationError)
    .attach("v7: add lifecycle_script_state column to sessions")?;
    Ok(())
}

/// v8: Add `metadata` column to sessions.
///
/// Stores a JSON blob of all session metadata. This eliminates the need
/// for individual columns per field - new fields on `SessionCore` are
/// automatically persisted via serde.
pub fn migrate_v8(conn: &mut rusqlite::Connection) -> Result<(), Report<SchemaMigrationError>> {
    conn.execute_batch("ALTER TABLE sessions ADD COLUMN metadata TEXT")
        .change_context(SchemaMigrationError)
        .attach("v8: add metadata column to sessions")?;
    Ok(())
}

/// v9: Rename `session_entries` table to `session_history`.
///
/// The old name was ambiguous - it sounded like a table of sessions.
/// The new name makes it clear this is the chat history junction table.
pub fn migrate_v9(conn: &mut rusqlite::Connection) -> Result<(), Report<SchemaMigrationError>> {
    conn.execute_batch("ALTER TABLE session_entries RENAME TO session_history")
        .change_context(SchemaMigrationError)
        .attach("v9: rename session_entries to session_history")?;
    Ok(())
}

/// v10: Consolidate all sessions to compaction-only strategy.
///
/// Rewrites `strategy_state` JSON to remove non-compaction keys
/// and sets `profile.strategy` to `"compaction"` for all sessions.
/// This prepares the database for the removal of other strategy types
/// from the Rust codebase.
pub fn migrate_v10(conn: &mut rusqlite::Connection) -> Result<(), Report<SchemaMigrationError>> {
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
        .change_context(SchemaMigrationError)
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
pub fn migrate_v11(conn: &mut rusqlite::Connection) -> Result<(), Report<SchemaMigrationError>> {
    conn.execute_batch(
        "ALTER TABLE sessions ADD COLUMN is_workflow BOOLEAN NOT NULL DEFAULT FALSE",
    )
    .change_context(SchemaMigrationError)
    .attach("v11: add is_workflow column to sessions")?;
    Ok(())
}

/// v12: Add `context_override` column to session_history.
///
/// Replaces the boolean `ignored` column with a tri-state text column:
/// `'default'`, `'forced_include'`, `'forced_exclude'`. The old `ignored`
/// column is kept for backward compatibility - the new column takes precedence.
/// Rows with `ignored = 1` are migrated to `'forced_exclude'`.
pub fn migrate_v12(conn: &mut rusqlite::Connection) -> Result<(), Report<SchemaMigrationError>> {
    conn.execute_batch(
        "ALTER TABLE session_history ADD COLUMN context_override TEXT NOT NULL DEFAULT 'default'",
    )
    .change_context(SchemaMigrationError)
    .attach("v12: add context_override column to session_history")?;
    conn.execute_batch(
        "UPDATE session_history SET context_override = 'forced_exclude' WHERE ignored = 1",
    )
    .change_context(SchemaMigrationError)
    .attach("v12: migrate ignored values to context_override")?;
    Ok(())
}

/// v13: Add `judge_meta` column to sessions.
///
/// Stores judge metadata as a nullable JSON text blob.
/// When NULL, the session is not a judge session.
pub fn migrate_v13(conn: &mut rusqlite::Connection) -> Result<(), Report<SchemaMigrationError>> {
    conn.execute_batch("ALTER TABLE sessions ADD COLUMN judge_meta TEXT")
        .change_context(SchemaMigrationError)
        .attach("v13: add judge_meta column to sessions")?;
    Ok(())
}

/// v14: Add `context_history` column to entries.
///
/// Stores the audit trail of context inclusion/exclusion changes as a JSON array
/// of `ContextChangeEvent`. Defaults to `'[]'` (empty audit) for existing rows.
pub fn migrate_v14(conn: &mut rusqlite::Connection) -> Result<(), Report<SchemaMigrationError>> {
    conn.execute_batch("ALTER TABLE entries ADD COLUMN context_history TEXT NOT NULL DEFAULT '[]'")
        .change_context(SchemaMigrationError)
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
pub fn migrate_v15(conn: &mut rusqlite::Connection) -> Result<(), Report<SchemaMigrationError>> {
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
    .change_context(SchemaMigrationError)
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
    .change_context(SchemaMigrationError)
    .attach("v15: copy sessions into sessions_new")?;

    conn.execute_batch("DROP TABLE sessions")
        .change_context(SchemaMigrationError)
        .attach("v15: drop old sessions table")?;

    conn.execute_batch("ALTER TABLE sessions_new RENAME TO sessions")
        .change_context(SchemaMigrationError)
        .attach("v15: rename sessions_new to sessions")?;

    Ok(())
}

pub fn migrate_v16(conn: &mut rusqlite::Connection) -> Result<(), Report<SchemaMigrationError>> {
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
    .change_context(SchemaMigrationError)
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
    .change_context(SchemaMigrationError)
    .attach("v16: copy sessions (is_workflow→is_automated, persist=TRUE)")?;

    conn.execute_batch("DROP TABLE sessions")
        .change_context(SchemaMigrationError)
        .attach("v16: drop old sessions table")?;

    conn.execute_batch("ALTER TABLE sessions_new RENAME TO sessions")
        .change_context(SchemaMigrationError)
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
pub fn migrate_v17(conn: &mut rusqlite::Connection) -> Result<(), Report<SchemaMigrationError>> {
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
        .change_context(SchemaMigrationError)
        .attach("v17: update session profile")?;
    }

    // Idempotent: ignore "duplicate column" error if migration runs twice.
    match conn.execute("ALTER TABLE token_ledger ADD COLUMN model_used TEXT", []) {
        Ok(_) => {}
        Err(rusqlite::Error::SqliteFailure(_, Some(msg)))
            if msg.contains("duplicate column name") => {}
        Err(e) => {
            return Err(e)
                .change_context(SchemaMigrationError)
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
pub fn migrate_v18(conn: &mut rusqlite::Connection) -> Result<(), Report<SchemaMigrationError>> {
    conn.execute_batch("ALTER TABLE entries RENAME COLUMN timestamp TO timing")
        .change_context(SchemaMigrationError)
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
pub fn migrate_v19(conn: &mut rusqlite::Connection) -> Result<(), Report<SchemaMigrationError>> {
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
        .change_context(SchemaMigrationError)
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
/// `PersistableCoreV20` snapshot from the columns, then rebuilds `sessions`
/// without them. After v20, every row has a metadata blob and the columns are gone.
///
/// `judge_meta` (vestigial — referenced only by an orphaned doc comment) is
/// dropped in the same rebuild.
pub fn migrate_v20(conn: &mut rusqlite::Connection) -> Result<(), Report<SchemaMigrationError>> {
    backfill_missing_metadata(conn)?;
    rebuild_sessions_without_zombies(conn)?;
    Ok(())
}

/// Reconstructs a `metadata` blob for every row where it is `NULL`.
///
/// Mirrors the pre-v20 legacy load branch: deserialize
/// `profile`/`blobs`/`lifecycle_args`/`lifecycle_script_state`
/// from their column JSON, parse `updated_at`/`created_at`/`parent_session`,
/// and serialize a full [`crate::PersistableCoreV20`].
fn backfill_missing_metadata(
    conn: &mut rusqlite::Connection,
) -> Result<(), Report<SchemaMigrationError>> {
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
        .change_context(SchemaMigrationError)
        .attach("v20: backfill metadata")?;
    }

    Ok(())
}

/// 12-step SQLite table rebuild dropping the zombie + `judge_meta` columns.
///
/// `sessions` has no indexes beyond its implicit PK, so none need recreating.
fn rebuild_sessions_without_zombies(
    conn: &mut rusqlite::Connection,
) -> Result<(), Report<SchemaMigrationError>> {
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
    .change_context(SchemaMigrationError)
    .attach("v20: create sessions_new without zombie columns")?;

    conn.execute_batch(
        "INSERT INTO sessions_new (\
         id, title, updated_at, created_at, parent_session, archived, metadata,\
         is_automated, persist) \
         SELECT id, title, updated_at, created_at, parent_session, archived, metadata,\
         is_automated, persist FROM sessions",
    )
    .change_context(SchemaMigrationError)
    .attach("v20: copy sessions into sessions_new")?;

    conn.execute_batch("DROP TABLE sessions")
        .change_context(SchemaMigrationError)
        .attach("v20: drop old sessions table")?;

    conn.execute_batch("ALTER TABLE sessions_new RENAME TO sessions")
        .change_context(SchemaMigrationError)
        .attach("v20: rename sessions_new to sessions")?;

    Ok(())
}

/// v21: Add the `discord_thread` mapping table.
///
/// The Discord bot frontend persists a 1:1 mapping between a Discord forum
/// thread and a jinn session id, so a thread can resume its session across
/// bot restarts (and auto-un-archive on inbound message). This table is
/// owned entirely by the discord layer; its columns are independent of the
/// session schema.
pub fn migrate_v21(conn: &mut rusqlite::Connection) -> Result<(), Report<SchemaMigrationError>> {
    conn.execute_batch(
        "CREATE TABLE discord_thread (\
         thread_id  TEXT PRIMARY KEY,\
         session_id TEXT NOT NULL,\
         guild_id   TEXT,\
         created_at INTEGER NOT NULL)",
    )
    .change_context(SchemaMigrationError)
    .attach("v21: create discord_thread mapping table")?;

    Ok(())
}

/// v22: Add the `entry_blobs` table for multimodal attachments.
///
/// Each row stores one attachment blob (raw bytes + media type) keyed by
/// `(entry_id, ordinal)`, where `ordinal` is the position of the attachment
/// within its parent entry. The `entries.kind` JSON no longer needs to carry
/// base64 image data, keeping the JSON column lean. Rows cascade with the
/// parent entry via `ON DELETE CASCADE`.
pub fn migrate_v22(conn: &mut rusqlite::Connection) -> Result<(), Report<SchemaMigrationError>> {
    conn.execute_batch(
        "CREATE TABLE entry_blobs (\
         entry_id TEXT NOT NULL REFERENCES entries(id) ON DELETE CASCADE,\
         ordinal INTEGER NOT NULL,\
         media_type TEXT NOT NULL,\
         data BLOB NOT NULL,\
         PRIMARY KEY (entry_id, ordinal))",
    )
    .change_context(SchemaMigrationError)
    .attach("v22: create entry_blobs table")?;

    Ok(())
}

/// v23: Strip the legacy `s-` prefix from session-id strings.
///
/// Historically [`SessionId`] stored its UUID with an `s-` prefix in
/// `Display` (e.g. `s-0195a3b2-...`). The newtype now exposes the bare
/// UUID and `Serialize`/`Deserialize` already round-trip bare values, so
/// the prefix survives only in data written by older jinn versions. This
/// migration rewrites every persisted session-id location to the bare
/// UUID form so the in-memory representation matches the persisted one.
///
/// Locations rewritten:
/// - `sessions.id` (PK)
/// - `sessions.parent_session` (nullable FK)
/// - `session_history.session_id`
/// - `token_ledger.session_id`
/// - `discord_thread.session_id`
/// - `sessions.metadata` JSON blob keys `session_id` and `parent_session`
///
/// Each column is updated only where its value begins with `s-`
/// (`substr(col, 1, 2) = 's-'`), so already-bare values (and any
/// non-conforming legacy strings) pass through untouched. The blob rewrite
/// is fault-tolerant: an unparseable blob is left unchanged (a migration
/// must never block on data an older system could have produced).
///
/// Runs under `PRAGMA foreign_keys=OFF` (the runner toggles it), so the
/// `sessions.id` rewrite does not fire cascades on the child tables — they
/// are updated explicitly below.
pub fn migrate_v23(conn: &mut rusqlite::Connection) -> Result<(), Report<SchemaMigrationError>> {
    // SQL columns — strip the prefix only where present.
    conn.execute_batch("UPDATE sessions SET id = substr(id, 3) WHERE substr(id, 1, 2) = 's-'")
        .change_context(SchemaMigrationError)
        .attach("v23: strip s- prefix from sessions.id")?;

    conn.execute_batch(
        "UPDATE sessions SET parent_session = substr(parent_session, 3) WHERE parent_session IS NOT NULL AND substr(parent_session, 1, 2) = 's-'",
    )
    .change_context(SchemaMigrationError)
    .attach("v23: strip s- prefix from sessions.parent_session")?;

    conn.execute_batch(
        "UPDATE session_history SET session_id = substr(session_id, 3) WHERE substr(session_id, 1, 2) = 's-'",
    )
    .change_context(SchemaMigrationError)
    .attach("v23: strip s- prefix from session_history.session_id")?;

    conn.execute_batch(
        "UPDATE token_ledger SET session_id = substr(session_id, 3) WHERE substr(session_id, 1, 2) = 's-'",
    )
    .change_context(SchemaMigrationError)
    .attach("v23: strip s- prefix from token_ledger.session_id")?;

    conn.execute_batch(
        "UPDATE discord_thread SET session_id = substr(session_id, 3) WHERE substr(session_id, 1, 2) = 's-'",
    )
    .change_context(SchemaMigrationError)
    .attach("v23: strip s- prefix from discord_thread.session_id")?;

    // JSON blob — rewrite the session_id / parent_session keys.
    rewrite_metadata_session_ids(conn)?;

    Ok(())
}

/// v24: Add provider-reported prompt/cache token columns to `token_ledger`.
///
/// Adds `prompt_tokens` (provider-reported prompt count, distinct from the
/// local estimate `tokens_sent`) and `cached_tokens` (cache-hit count from
/// `usage.prompt_tokens_details.cached_tokens`). Both nullable: pre-v24 rows
/// and turns without provider usage (e.g. cancelled) load as `NULL`.
pub fn migrate_v24(conn: &mut rusqlite::Connection) -> Result<(), Report<SchemaMigrationError>> {
    for col in ["prompt_tokens", "cached_tokens"] {
        let sql = format!("ALTER TABLE token_ledger ADD COLUMN {col} INTEGER");
        // Idempotent: ignore "duplicate column" error if migration runs twice.
        match conn.execute(&sql, []) {
            Ok(_) => {}
            Err(rusqlite::Error::SqliteFailure(_, Some(msg)))
                if msg.contains("duplicate column name") => {}
            Err(e) => {
                return Err(e)
                    .change_context(SchemaMigrationError)
                    .attach(format!("v24: add {col} column to token_ledger"));
            }
        }
    }
    Ok(())
}

/// Rewrites the `session_id` and `parent_session` string values inside the
/// `sessions.metadata` JSON blob, stripping a leading `s-` where present.
///
/// Fault-tolerant: rows whose `metadata` is NULL, empty, unparseable as an
/// object, or whose targeted keys are not strings are left unchanged.
fn rewrite_metadata_session_ids(
    conn: &mut rusqlite::Connection,
) -> Result<(), Report<SchemaMigrationError>> {
    let mut stmt = conn
        .prepare("SELECT rowid, metadata FROM sessions WHERE metadata IS NOT NULL")
        .change_context(SchemaMigrationError)
        .attach("v23: prepare metadata scan")?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .change_context(SchemaMigrationError)
        .attach("v23: scan metadata rows")?;
    for row in rows {
        let (rowid, raw) = row
            .change_context(SchemaMigrationError)
            .attach("v23: read metadata row")?;
        let rewritten = strip_prefix_in_blob(&raw);
        if rewritten != raw {
            conn.execute(
                "UPDATE sessions SET metadata = ? WHERE rowid = ?",
                rusqlite::params![rewritten, rowid],
            )
            .change_context(SchemaMigrationError)
            .attach("v23: rewrite metadata blob")?;
        }
    }
    Ok(())
}

/// Strips a leading `s-` from the `session_id` and `parent_session` string
/// values inside a `metadata` JSON object. Returns the input verbatim if the
/// JSON is unparseable, not an object, or the keys are absent/non-string.
fn strip_prefix_in_blob(raw: &str) -> String {
    let mut map: serde_json::Map<String, serde_json::Value> = match serde_json::from_str(raw) {
        Ok(serde_json::Value::Object(m)) => m,
        _ => return raw.to_owned(),
    };
    let mut changed = false;
    for key in ["session_id", "parent_session"] {
        if let Some(serde_json::Value::String(s)) = map.get_mut(key) {
            if let Some(rest) = s.strip_prefix('s').and_then(|t| t.strip_prefix('-')) {
                *s = rest.to_owned();
                changed = true;
            }
        }
    }
    if changed {
        serde_json::to_string(&serde_json::Value::Object(map)).unwrap_or_else(|_| raw.to_owned())
    } else {
        raw.to_owned()
    }
}

/// A legacy `sessions` row whose `metadata` is NULL — the pre-v8 shape that
/// the backfill reconstructs a [`crate::PersistableCoreV20`] blob from.
struct LegacyRow {
    /// SQLite rowid — the stable key for the subsequent UPDATE.
    rowid: i64,
    /// The pre-v8 column values, verbatim.
    columns: LegacySessionColumns,
}

impl LegacyRow {
    fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Self {
            rowid: row.get(0)?,
            columns: LegacySessionColumns {
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

    /// Reconstructs the [`crate::PersistableCoreV20`] blob from the legacy columns,
    /// exactly mirroring the pre-v20 legacy load branch.
    ///
    /// # Errors
    ///
    /// Returns an error if any column JSON fails to deserialize or the
    /// reconstructed blob cannot be serialized.
    fn metadata_blob(&self) -> Result<String, Report<SchemaMigrationError>> {
        crate::PersistableCoreV20::blob_from_legacy_columns(&self.columns)
    }
}

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
) -> Result<Vec<T>, Report<SchemaMigrationError>>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    let mut stmt = conn
        .prepare(sql)
        .change_context(SchemaMigrationError)
        .attach(format!("{tag}: prepare statement"))?;
    let mapped = stmt
        .query_map([], map)
        .change_context(SchemaMigrationError)
        .attach(format!("{tag}: map rows"))?;
    let mut rows = Vec::new();
    for row in mapped {
        rows.push(
            row.change_context(SchemaMigrationError)
                .attach(format!("{tag}: collect row"))?,
        );
    }
    Ok(rows)
}

// ── Test helpers ─────────────────────────────────────────────────────────
//
// These mirror jinn-domain's test-only `apply_migrations_inner` and
// `seed_at_version` so the schema crate's own tests and (transitively)
// jinn-domain's tests can stand up a DB at a specific legacy version.

#[cfg(feature = "testing")]
pub fn apply_migrations_inner(conn: &mut rusqlite::Connection, target: i32) {
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
    if target >= 21 {
        migrate_v21(conn).expect("v21");
        record_version(conn, 21, "add_discord_thread_table").expect("record v21");
    }
    if target >= 22 {
        migrate_v22(conn).expect("v22");
        record_version(conn, 22, "add_entry_blobs_table").expect("record v22");
    }
    if target >= 23 {
        migrate_v23(conn).expect("v23");
        record_version(conn, 23, "strip_s_prefix_from_session_ids").expect("record v23");
    }
}

/// Test-only: applies migrations up to (and including) `target` on a held
/// connection, recording each version.
///
/// Mirrors jinn-domain's test-only helper so the schema crate's own tests can
/// stand up a DB at a specific legacy version. FK=OFF is the caller's job
/// (the public [`crate::run_migrations`] toggles it; direct test callers
/// toggle it themselves).
#[cfg(feature = "testing")]
#[allow(dead_code)]
pub fn apply_up_to_no_fk(conn: &mut rusqlite::Connection, target: i32) {
    bootstrap_tracking_table(conn).expect("bootstrap");
    apply_migrations_inner(conn, target);
}

/// Test-only re-exports of individual migrations and helpers.
///
/// Downstream test suites (e.g. jinn-domain's migrator tests) need to stand a
/// DB up at a specific version and assert against a single migration's
#[cfg(feature = "testing")]
#[allow(unused_imports)]
pub mod testing {
    pub use super::{
        apply_migrations_inner, bootstrap_tracking_table, migrate_v10, migrate_v15, migrate_v16,
        migrate_v17, migrate_v18, migrate_v19, migrate_v21, migrate_v22, migrate_v23,
        record_version,
    };
}
