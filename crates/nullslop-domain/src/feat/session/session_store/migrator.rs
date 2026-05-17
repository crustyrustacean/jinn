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

/// Runs all pending database migrations.
///
/// Bootstraps the `_migrations` tracking table, reads the current version,
/// and runs any migrations that haven't been applied yet.
///
/// # Panics
///
/// Panics if any migration fails — the database is left in whatever state
/// the failed migration produced. This is intentional: silent migration
/// failures are worse than loud ones.
pub fn run_migrations(conn: &mut SqliteConnection) {
    bootstrap_tracking_table(conn);
    let current = current_version(conn);

    if current < 0 {
        migrate_v0(conn);
        record_version(conn, 0, "create_initial_schema");
    }
    if current < 1 {
        migrate_v1(conn);
        record_version(conn, 1, "add_cwd_column");
    }
    if current < 2 {
        migrate_v2(conn);
        record_version(conn, 2, "add_created_at_column");
    }
}

// ── Tracking table ───────────────────────────────────────────────────────

/// Creates the `_migrations` tracking table.
///
/// This is the only place `IF NOT EXISTS` is used — the tracking table must
/// bootstrap itself before version checking can begin. All other migrations
/// use strict DDL so failures are loud.
fn bootstrap_tracking_table(conn: &mut SqliteConnection) {
    sql_query(
        "CREATE TABLE IF NOT EXISTS _migrations (\
         version INTEGER NOT NULL,\
         name TEXT NOT NULL,\
         applied_at TEXT NOT NULL DEFAULT (datetime('now')))",
    )
    .execute(conn)
    .expect("failed to create _migrations table");
}

/// Row returned by the version query.
#[derive(QueryableByName)]
struct VersionRow {
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
    version: Option<i32>,
}

/// Reads the highest migration version from the tracking table.
///
/// Returns -1 if no migrations have been recorded (empty database).
fn current_version(conn: &mut SqliteConnection) -> i32 {
    let result: Vec<VersionRow> = sql_query("SELECT MAX(version) AS version FROM _migrations")
        .load(conn)
        .expect("failed to query migration version");
    result.first().and_then(|r| r.version).unwrap_or(-1)
}

/// Records a completed migration in the tracking table.
fn record_version(conn: &mut SqliteConnection, version: i32, name: &str) {
    sql_query("INSERT INTO _migrations (version, name) VALUES (?, ?)")
        .bind::<diesel::sql_types::Integer, _>(version)
        .bind::<diesel::sql_types::Text, _>(name)
        .execute(conn)
        .unwrap_or_else(|e| panic!("failed to record migration v{version}: {e}"));
}

// ── Migrations ───────────────────────────────────────────────────────────

/// v0: Initial schema — sessions, entries, session_entries, token_ledger.
fn migrate_v0(conn: &mut SqliteConnection) {
    sql_query(
        "CREATE TABLE sessions (\
         id TEXT PRIMARY KEY,\
         title TEXT,\
         updated_at TEXT NOT NULL,\
         profile TEXT NOT NULL DEFAULT '{}',\
         strategy_state TEXT NOT NULL DEFAULT '{}',\
         blobs TEXT NOT NULL DEFAULT '{}',\
         parent_session TEXT DEFAULT NULL)",
    )
    .execute(conn)
    .expect("v0: create sessions table");

    sql_query(
        "CREATE TABLE entries (\
         id TEXT PRIMARY KEY,\
         timestamp TEXT NOT NULL,\
         kind TEXT NOT NULL)",
    )
    .execute(conn)
    .expect("v0: create entries table");

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
    .expect("v0: create session_entries table");

    sql_query(
        "CREATE TABLE token_ledger (\
         id INTEGER PRIMARY KEY AUTOINCREMENT,\
         session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,\
         timestamp TEXT NOT NULL,\
         tokens_sent INTEGER NOT NULL,\
         tokens_received INTEGER NOT NULL)",
    )
    .execute(conn)
    .expect("v0: create token_ledger table");

    sql_query(
        "CREATE INDEX idx_session_entries_session ON session_entries(session_id, ordinal)",
    )
    .execute(conn)
    .expect("v0: create session_entries index");

    sql_query("CREATE INDEX idx_token_ledger_session ON token_ledger(session_id)")
        .execute(conn)
        .expect("v0: create token_ledger index");
}

/// v1: Add `cwd` column to sessions.
fn migrate_v1(conn: &mut SqliteConnection) {
    sql_query("ALTER TABLE sessions ADD COLUMN cwd TEXT NOT NULL DEFAULT '.'")
        .execute(conn)
        .expect("v1: add cwd column to sessions");
}

/// v2: Add `created_at` column to sessions.
fn migrate_v2(conn: &mut SqliteConnection) {
    sql_query("ALTER TABLE sessions ADD COLUMN created_at TEXT NOT NULL DEFAULT ''")
        .execute(conn)
        .expect("v2: add created_at column to sessions");
}

#[cfg(test)]
mod tests {
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
        // Given a fresh database.
        let (_dir, mut conn) = make_conn();

        // When running migrations.
        run_migrations(&mut conn);

        // Then the _migrations table has 3 entries.
        #[derive(QueryableByName)]
        struct MigrationRow {
            #[diesel(sql_type = diesel::sql_types::Integer)]
            version: i32,
            #[diesel(sql_type = diesel::sql_types::Text)]
            name: String,
        }

        let rows: Vec<MigrationRow> =
            sql_query("SELECT version, name FROM _migrations ORDER BY version")
                .load(&mut conn)
                .expect("query migrations");

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].version, 0);
        assert_eq!(rows[0].name, "create_initial_schema");
        assert_eq!(rows[1].version, 1);
        assert_eq!(rows[1].name, "add_cwd_column");
        assert_eq!(rows[2].version, 2);
        assert_eq!(rows[2].name, "add_created_at_column");
    }

    #[test]
    fn re_running_migrations_is_noop() {
        // Given a database with migrations already applied.
        let (_dir, mut conn) = make_conn();
        run_migrations(&mut conn);

        // When running migrations again.
        run_migrations(&mut conn);

        // Then no duplicate entries are added.
        #[derive(QueryableByName)]
        struct CountRow {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            count: i64,
        }

        let rows: Vec<CountRow> =
            sql_query("SELECT COUNT(*) AS count FROM _migrations")
                .load(&mut conn)
                .expect("query count");

        assert_eq!(rows[0].count, 3);
    }

    #[test]
    fn fresh_database_has_all_tables() {
        // Given a fresh database.
        let (_dir, mut conn) = make_conn();

        // When running migrations.
        run_migrations(&mut conn);

        // Then all expected tables exist.
        #[derive(QueryableByName)]
        struct TableRow {
            #[diesel(sql_type = diesel::sql_types::Text)]
            name: String,
        }

        let tables: Vec<TableRow> =
            sql_query("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                .load(&mut conn)
                .expect("query tables");
        let table_names: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();

        assert!(table_names.contains(&"_migrations"));
        assert!(table_names.contains(&"entries"));
        assert!(table_names.contains(&"session_entries"));
        assert!(table_names.contains(&"sessions"));
        assert!(table_names.contains(&"token_ledger"));
    }
}
