//! Verifies the schema crate produces the expected post-v20 shape standalone.

use jinn_session_schema::run_migrations;

/// `_migrations` tracking row at v25.
#[rstest::rstest]
#[test]
fn fresh_database_has_all_tables_and_v21() {
    // Given a fresh in-memory database.
    let mut conn = rusqlite::Connection::open_in_memory().expect("open in-memory db");

    // When running all migrations.
    run_migrations(&mut conn).expect("run migrations");

    // Then all eight application tables exist.
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
    assert!(
        tables.contains(&"session_fts".to_owned()),
        "session_fts table missing: {tables:?}"
    );
    assert!(
        tables.contains(&"fts_dirty".to_owned()),
        "fts_dirty table missing: {tables:?}"
    );

    // And the highest recorded migration version is 26.
    let version: i64 = conn
        .query_row("SELECT MAX(version) FROM _migrations", [], |row| row.get(0))
        .expect("query version");
    assert_eq!(version, 26, "migration version");

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

/// v25 adds the nullable `entries.token_count` column. Legacy rows (pre-v25)
/// read back as NULL, which the domain maps to "not yet computed".
#[rstest::rstest]
#[test]
#[cfg(feature = "testing")]
fn v25_adds_nullable_token_count_column_to_entries() {
    use jinn_session_schema::testing::{apply_migrations_inner, bootstrap_tracking_table};

    // Given a DB migrated only to v24, seeded with one entry.
    let mut conn = rusqlite::Connection::open_in_memory().expect("open db");
    bootstrap_tracking_table(&mut conn).expect("bootstrap");
    apply_migrations_inner(&mut conn, 24);
    conn.execute(
        "INSERT INTO entries (id, timing, kind) VALUES ('e-1', '2024-01-01T00:00:00Z', '{}')",
        [],
    )
    .expect("insert entry");

    // When running the pending migrations (v25).
    run_migrations(&mut conn).expect("run pending migrations");

    // Then the token_count column exists, is NULL for the legacy row, and
    // accepts non-NULL counts.
    let columns: Vec<(String, String)> = conn
        .prepare("PRAGMA table_info(entries)")
        .expect("prepare")
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    let token_count = columns
        .iter()
        .find(|(name, _)| name == "token_count")
        .expect("entries.token_count column missing");
    assert_eq!(
        token_count.1.to_uppercase(),
        "INTEGER",
        "token_count must be INTEGER, got: {token_count:?}"
    );

    let stored: Option<Option<i64>> = conn
        .query_row(
            "SELECT token_count FROM entries WHERE id = 'e-1'",
            [],
            |row| row.get(0),
        )
        .expect("select legacy row");
    assert!(
        stored.is_none(),
        "legacy row's token_count must read back as NULL"
    );

    conn.execute("UPDATE entries SET token_count = 42 WHERE id = 'e-1'", [])
        .expect("set token count");
    let after: i64 = conn
        .query_row(
            "SELECT token_count FROM entries WHERE id = 'e-1'",
            [],
            |row| row.get(0),
        )
        .expect("select updated row");
    assert_eq!(after, 42);
}

/// v26 seeds `fts_dirty` with every session that exists at migration time, so
/// the first post-upgrade launch backfills the FTS index in the background.
#[rstest::rstest]
#[test]
#[cfg(feature = "testing")]
fn v26_seeds_every_existing_session_dirty() {
    use jinn_session_schema::testing::{apply_migrations_inner, bootstrap_tracking_table};

    // Given a DB migrated only to v25 with two sessions in it.
    let mut conn = rusqlite::Connection::open_in_memory().expect("open db");
    bootstrap_tracking_table(&mut conn).expect("bootstrap");
    apply_migrations_inner(&mut conn, 25);
    conn.execute(
        "INSERT INTO sessions (id, title, updated_at, created_at) VALUES ('s-a', 'A', 't', 't')",
        [],
    )
    .expect("insert session a");
    conn.execute(
        "INSERT INTO sessions (id, title, updated_at, created_at) VALUES ('s-b', 'B', 't', 't')",
        [],
    )
    .expect("insert session b");

    // When running the pending migrations (v26).
    run_migrations(&mut conn).expect("run pending migrations");

    // Then both session ids are marked dirty.
    let dirty: Vec<String> = conn
        .prepare("SELECT session_id FROM fts_dirty ORDER BY session_id")
        .expect("prepare")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    assert_eq!(dirty, vec!["s-a".to_owned(), "s-b".to_owned()]);
}

/// v26's dirty-marking triggers: INSERT, UPDATE, and DELETE on `sessions` each
/// mark the affected session id in `fts_dirty` (deduplicated by the PK).
#[rstest::rstest]
#[test]
#[cfg(feature = "testing")]
fn v26_triggers_mark_sessions_dirty_on_insert_update_delete() {
    // Given a fully-migrated DB.
    let mut conn = rusqlite::Connection::open_in_memory().expect("open db");
    run_migrations(&mut conn).expect("run migrations");

    // When inserting a session.
    conn.execute(
        "INSERT INTO sessions (id, title, updated_at, created_at) VALUES ('s-a', 'A', 't1', 't1')",
        [],
    )
    .expect("insert session");

    // Then the insert trigger marks it dirty.
    let dirty: i64 = conn
        .query_row("SELECT COUNT(*) FROM fts_dirty WHERE session_id = 's-a'", [], |row| {
            row.get(0)
        })
        .expect("count dirty after insert");
    assert_eq!(dirty, 1, "insert must mark dirty");

    // When updating that session (the save path upserts the row every save).
    conn.execute("UPDATE sessions SET title = 'A2' WHERE id = 's-a'", [])
        .expect("update session");

    // Then the update trigger marks it dirty, and the PRIMARY KEY dedupes
    // repeated marks into a single row.
    let dirty: i64 = conn
        .query_row("SELECT COUNT(*) FROM fts_dirty", [], |row| row.get(0))
        .expect("count dirty after update");
    assert_eq!(dirty, 1, "update must not duplicate the dirty row");

    // When inserting a second session (to prove DELETE only marks its own id)
    // and then deleting the first.
    conn.execute(
        "INSERT INTO sessions (id, title, updated_at, created_at) VALUES ('s-b', 'B', 't2', 't2')",
        [],
    )
    .expect("insert second session");
    conn.execute("DELETE FROM sessions WHERE id = 's-a'", []).expect("delete session");

    // Then the delete trigger marks the deleted id dirty.
    let dirty: Vec<String> = conn
        .prepare("SELECT session_id FROM fts_dirty ORDER BY session_id")
        .expect("prepare")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    assert_eq!(dirty, vec!["s-a".to_owned(), "s-b".to_owned()]);
}

/// v26's `session_fts` virtual table is functional in the build DB: MATCH
/// queries work, UNINDEXED columns store but never match, and `snippet()`
/// wraps matches. (Catches a system-sqlite lacking FTS5 at build time.)
#[rstest::rstest]
#[test]
fn v26_fts_table_accepts_rows_and_matches() {
    // Given a fully-migrated DB.
    let mut conn = rusqlite::Connection::open_in_memory().expect("open db");
    run_migrations(&mut conn).expect("run migrations");

    // When inserting two FTS rows with UNINDEXED metadata.
    conn.execute(
        "INSERT INTO session_fts(body, role, session_id, entry_id, entry_ts) \
         VALUES ('the parser rewrites the junction table', 'assistant', 's-a', 'e-1', \
         '2026-09-01T00:00:00Z')",
        [],
    )
    .expect("insert fts row 1");
    conn.execute(
        "INSERT INTO session_fts(body, role, session_id, entry_id, entry_ts) \
         VALUES ('something entirely unrelated to the query terms', 'assistant', 's-b', 'e-2', \
         '2026-09-02T00:00:00Z')",
        [],
    )
    .expect("insert fts row 2");

    // Then a MATCH on 'parser' hits exactly the first row.
    let hits: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM session_fts WHERE session_fts MATCH 'parser'",
            [],
            |row| row.get(0),
        )
        .expect("match parser");
    assert_eq!(hits, 1, "MATCH 'parser' must hit only row 1");

    // And the term 'assistant' never matches via body because the role column
    // is UNINDEXED (bare terms can't match filter columns).
    let hits: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM session_fts WHERE session_fts MATCH 'assistant'",
            [],
            |row| row.get(0),
        )
        .expect("match assistant");
    assert_eq!(hits, 0, "UNINDEXED role words must never match");

    // And porter stemming folds 'rewrites' into 'rewrite'.
    let hits: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM session_fts WHERE session_fts MATCH 'rewrite'",
            [],
            |row| row.get(0),
        )
        .expect("match rewrite");
    assert_eq!(hits, 1, "porter stemmer must fold rewrites/rewrite");

    // And snippet() wraps the matched terms.
    let snip: String = conn
        .query_row(
            "SELECT snippet(session_fts, 0, '<<', '>>', ' … ', 12) FROM session_fts \
             WHERE session_fts MATCH 'parser'",
            [],
            |row| row.get(0),
        )
        .expect("snippet");
    assert!(
        snip.contains("<<parser>>"),
        "snippet must wrap the match, got: {snip}"
    );
}
