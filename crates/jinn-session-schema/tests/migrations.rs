//! Verifies the schema crate produces the expected post-v20 shape standalone.

use jinn_session_schema::run_migrations;

/// `_migrations` tracking row at v24.
#[rstest::rstest]
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
    assert!(
        tables.contains(&"entry_blobs".to_owned()),
        "entry_blobs table missing: {tables:?}"
    );

    // And the highest recorded migration version is 24.
    let version: i64 = conn
        .query_row("SELECT MAX(version) FROM _migrations", [], |row| row.get(0))
        .expect("query version");
    assert_eq!(version, 24, "migration version");

    // And token_ledger has the v24 prompt/cache columns.
    let columns: Vec<String> = conn
        .prepare("PRAGMA table_info(token_ledger)")
        .expect("prepare")
        .query_map([], |row| row.get::<_, String>(1))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    assert!(
        columns.contains(&"prompt_tokens".to_owned()),
        "token_ledger.prompt_tokens missing: {columns:?}"
    );
    assert!(
        columns.contains(&"cached_tokens".to_owned()),
        "token_ledger.cached_tokens missing: {columns:?}"
    );
}

/// Re-running `run_migrations` on a fully-migrated database is a no-op: the
/// version guards (`current < N`) prevent re-application.
#[rstest::rstest]
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
#[rstest::rstest]
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

/// v23 strips the legacy `s-` prefix from every persisted session-id
/// location: the five SQL columns plus the `session_id` and
/// `parent_session` keys inside the `sessions.metadata` JSON blob.
#[rstest::rstest]
#[test]
#[cfg(feature = "testing")]
fn v23_strips_s_prefix_from_all_session_id_locations() {
    // Given a fresh DB migrated only to v22, seeded with s-prefixed IDs
    // across all five SQL columns and a prefixed metadata blob.
    use jinn_session_schema::testing::{apply_migrations_inner, bootstrap_tracking_table};

    let mut conn = rusqlite::Connection::open_in_memory().expect("open db");
    bootstrap_tracking_table(&mut conn).expect("bootstrap");
    apply_migrations_inner(&mut conn, 22);

    // A valid UUID v4 we will prefix in the seeded rows.
    const CHILD: &str = "0195a3b2-4f8c-7d2e-8a1b-5c3d2e1f0a0b";
    const PARENT: &str = "0195a3b2-4f8c-7d2e-0000-000000000001";

    // sessions: one prefixed child + one prefixed parent. metadata blob
    // carries prefixed session_id and parent_session for the child.
    let child_blob = format!(
        "
        {{\"session_id\":\"s-{CHILD}\",\"parent_session\":\"s-{PARENT}\",
        \"profile\":{{}},\"blobs\":{{}},\"lifecycle_script_state\":\"nothing_ran\",
        \"lifecycle_args\":[],\"cwd\":\".\",\"persist\":true}}
    "
    );
    conn.execute(
        "INSERT INTO sessions (id, title, updated_at, created_at, parent_session, archived, metadata, is_automated, persist) VALUES (?, 'child', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z', ?, 0, ?, 0, 1)",
        rusqlite::params![format!("s-{CHILD}"), format!("s-{PARENT}"), child_blob],
    )
    .expect("insert child session");
    conn.execute(
        "INSERT INTO sessions (id, title, updated_at, created_at, parent_session, archived, metadata, is_automated, persist) VALUES (?, 'parent', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z', NULL, 0, NULL, 0, 1)",
        rusqlite::params![format!("s-{PARENT}")],
    )
    .expect("insert parent session");

    // session_history, token_ledger, discord_thread with prefixed child id.
    conn.execute(
        "INSERT INTO entries (id, timing, kind) VALUES ('e-1', '2024-01-01T00:00:00Z', '{}')",
        [],
    )
    .expect("insert entry");
    conn.execute(
        "INSERT INTO session_history (session_id, entry_id, ordinal) VALUES (?, 'e-1', 0)",
        [format!("s-{CHILD}")],
    )
    .expect("insert session_history");
    conn.execute(
        "INSERT INTO token_ledger (session_id, timestamp, tokens_sent, tokens_received) VALUES (?, '2024-01-01T00:00:00Z', 1, 1)",
        [format!("s-{CHILD}")],
    )
    .expect("insert token_ledger");
    conn.execute(
        "INSERT INTO discord_thread (thread_id, session_id, guild_id, created_at) VALUES ('t-1', ?, NULL, 0)",
        [format!("s-{CHILD}")],
    )
    .expect("insert discord_thread");
    // When running migrations (production path: toggles FK off, runs v23, re-enables + checks).
    run_migrations(&mut conn).expect("run pending migrations");

    // Then the five SQL columns are bare UUIDs.
    let sid: String = conn
        .query_row("SELECT id FROM sessions WHERE title = 'child'", [], |r| {
            r.get(0)
        })
        .expect("select child id");
    assert_eq!(sid, CHILD, "sessions.id should be bare");

    let parent_col: String = conn
        .query_row(
            "SELECT parent_session FROM sessions WHERE title = 'child'",
            [],
            |r| r.get(0),
        )
        .expect("select parent_session");
    assert_eq!(parent_col, PARENT, "sessions.parent_session should be bare");

    let hist: String = conn
        .query_row("SELECT session_id FROM session_history", [], |r| r.get(0))
        .expect("select history");
    assert_eq!(hist, CHILD, "session_history.session_id should be bare");

    let tl: String = conn
        .query_row("SELECT session_id FROM token_ledger", [], |r| r.get(0))
        .expect("select ledger");
    assert_eq!(tl, CHILD, "token_ledger.session_id should be bare");

    let dt: String = conn
        .query_row("SELECT session_id FROM discord_thread", [], |r| r.get(0))
        .expect("select discord");
    assert_eq!(dt, CHILD, "discord_thread.session_id should be bare");

    // And the metadata blob's session_id / parent_session are bare.
    let blob: String = conn
        .query_row(
            "SELECT metadata FROM sessions WHERE title = 'child'",
            [],
            |r| r.get(0),
        )
        .expect("select metadata");
    assert!(
        blob.contains(&format!("\"session_id\":\"{CHILD}\"")),
        "blob session_id should be bare, got: {blob}"
    );
    assert!(
        blob.contains(&format!("\"parent_session\":\"{PARENT}\"")),
        "blob parent_session should be bare, got: {blob}"
    );
}
