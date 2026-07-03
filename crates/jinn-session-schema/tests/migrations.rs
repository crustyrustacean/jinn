//! Verifies the schema crate produces the expected post-v20 shape standalone.

use jinn_session_schema::run_migrations;

/// A fresh database, after `run_migrations`, contains all six tables plus the
/// `_migrations` tracking row at v21.
#[test]
fn fresh_database_has_all_tables_and_v21() {
    // Given a fresh in-memory database.
    let mut conn = rusqlite::Connection::open_in_memory().expect("open in-memory db");

    // When running all migrations.
    run_migrations(&mut conn).expect("run migrations");

    // Then all six application tables exist.
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .expect("prepare")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();

    assert!(
        tables.contains(&"sessions".to_owned()),
        "sessions table missing: {tables:?}"
    );
    assert!(
        tables.contains(&"entries".to_owned()),
        "entries table missing: {tables:?}"
    );
    assert!(
        tables.contains(&"session_history".to_owned()),
        "session_history table missing: {tables:?}",
    );
    assert!(
        tables.contains(&"token_ledger".to_owned()),
        "token_ledger table missing: {tables:?}",
    );
    assert!(
        tables.contains(&"discord_thread".to_owned()),
        "discord_thread table missing: {tables:?}"
    );

    // And the highest recorded migration version is 21.
    let version: i64 = conn
        .query_row("SELECT MAX(version) FROM _migrations", [], |row| row.get(0))
        .expect("query version");
    assert_eq!(version, 21, "migration version");
}

/// Re-running `run_migrations` on a fully-migrated database is a no-op: the
/// version guards (`current < N`) prevent re-application.
#[test]
fn re_running_migrations_is_noop() {
    let mut conn = rusqlite::Connection::open_in_memory().expect("open in-memory db");
    run_migrations(&mut conn).expect("first run");

    // A row inserted before the second run must survive it.
    conn.execute(
        "INSERT INTO sessions (id, title, updated_at, created_at) VALUES ('x', 'X', 't', 't')",
        [],
    )
    .expect("insert sentinel");

    run_migrations(&mut conn).expect("second run");

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 1, "sentinel row survived the no-op re-run");
}

/// The v20 rebuild left `sessions` with exactly the nine authoritative columns.
#[test]
fn sessions_has_nine_authoritative_columns() {
    let mut conn = rusqlite::Connection::open_in_memory().expect("open in-memory db");
    run_migrations(&mut conn).expect("run migrations");

    let columns: Vec<String> = conn
        .prepare("PRAGMA table_info(sessions)")
        .expect("prepare")
        .query_map([], |row| row.get::<_, String>(1))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();

    assert_eq!(
        columns,
        vec![
            "id",
            "title",
            "updated_at",
            "created_at",
            "parent_session",
            "archived",
            "metadata",
            "is_automated",
            "persist",
        ],
        "sessions columns after v20",
    );
}
