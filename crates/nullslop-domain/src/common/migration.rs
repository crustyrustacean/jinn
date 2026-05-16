//! Generic database migration runner.
//!
//! Provides a versioned migration system for SQLite databases. Each migration
//! is a Rust function that receives a [`Connection`] and can perform arbitrary
//! DDL or DML. The runner tracks applied versions in a `schema_migrations`
//! table and applies pending migrations in order.
//!
//! This module is database-agnostic — it can migrate any SQLite database
//! managed by the application. Domain-specific migration lists live alongside
//! their respective stores.

use error_stack::{Report, ResultExt as _};
use rusqlite::Connection;
use wherror::Error;

/// Error type for migration operations.
#[derive(Debug, Error)]
#[error(debug)]
pub struct MigrationError;

/// A single schema migration.
///
/// Migrations are ordered by `version` (sequential, starting at 0).
/// Each migration has a human-readable `name` for logging and an `up` function
/// that performs the schema change.
pub struct Migration {
    /// Sequential version number (0, 1, 2, ...).
    pub version: u32,
    /// Human-readable name (e.g., "create_initial_schema").
    pub name: &'static str,
    /// The migration function. Receives a [`Connection`] and can perform
    /// arbitrary DDL or DML.
    pub up: fn(&Connection) -> Result<(), Report<MigrationError>>,
}

/// Runs all pending migrations against the given connection.
///
/// Creates the `schema_migrations` table if it does not exist, reads the
/// current version, and applies each pending migration in order within
/// a transaction. Each applied migration is recorded in `schema_migrations`.
///
/// If all migrations have already been applied, this is a no-op.
///
/// # Errors
///
/// Returns [`MigrationError`] if:
/// - The `schema_migrations` table cannot be created or read.
/// - Any migration function fails.
/// - A migration cannot be recorded.
pub fn run_migrations(
    conn: &Connection,
    migrations: &[Migration],
) -> Result<(), Report<MigrationError>> {
    // Create the tracking table.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version   INTEGER PRIMARY KEY,
            name      TEXT NOT NULL,
            applied_at TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )
    .change_context(MigrationError)
    .attach("failed to create schema_migrations table")?;

    // Read current version.
    let current_version: Option<u32> = conn
        .query_row(
            "SELECT MAX(version) FROM schema_migrations",
            [],
            |row| row.get(0),
        )
        .ok()
        .flatten();

    let start = current_version.map_or(0, |v| v + 1);

    // Apply pending migrations.
    for migration in migrations {
        if migration.version < start {
            continue;
        }

        let tx = conn
            .unchecked_transaction()
            .change_context(MigrationError)
            .attach("failed to begin migration transaction")?;

        (migration.up)(&tx)
            .attach(migration.name)
            .attach(format!("migration v{} failed", migration.version))?;

        tx.execute(
            "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
            rusqlite::params![migration.version, migration.name],
        )
        .change_context(MigrationError)
        .attach("failed to record migration")?;

        tx.commit()
            .change_context(MigrationError)
            .attach("failed to commit migration")?;

        tracing::info!(
            version = migration.version,
            name = migration.name,
            "applied migration"
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Creates an in-memory SQLite connection for testing.
    fn test_conn() -> Connection {
        Connection::open_in_memory().expect("in-memory connection")
    }

    /// Reads the count of applied migrations.
    fn applied_count(conn: &Connection) -> usize {
        conn.query_row(
            "SELECT COUNT(*) FROM schema_migrations",
            [],
            |row| row.get::<_, usize>(0),
        )
        .expect("count migrations")
    }

    /// Reads the max applied version (None if no migrations applied).
    fn max_version(conn: &Connection) -> Option<u32> {
        conn.query_row(
            "SELECT MAX(version) FROM schema_migrations",
            [],
            |row| row.get(0),
        )
        .ok()
        .flatten()
    }

    /// Checks if a table exists in the database.
    fn table_exists(conn: &Connection, name: &str) -> bool {
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            rusqlite::params![name],
            |row| row.get::<_, usize>(0),
        )
        .expect("check table exists")
            > 0
    }

    // --- Fresh DB applies all migrations ---

    #[rstest::rstest]
    fn fresh_db_applies_all_migrations() {
        // Given a fresh in-memory database with 2 migrations.
        let conn = test_conn();

        fn v0(conn: &Connection) -> Result<(), Report<MigrationError>> {
            conn.execute_batch("CREATE TABLE foo (id INTEGER PRIMARY KEY)")
                .change_context(MigrationError)?;
            Ok(())
        }
        fn v1(conn: &Connection) -> Result<(), Report<MigrationError>> {
            conn.execute_batch("ALTER TABLE foo ADD COLUMN name TEXT")
                .change_context(MigrationError)?;
            Ok(())
        }

        let migrations = vec![
            Migration { version: 0, name: "create_foo", up: v0 },
            Migration { version: 1, name: "add_name", up: v1 },
        ];

        // When running migrations.
        run_migrations(&conn, &migrations).expect("run migrations");

        // Then both migrations are recorded.
        assert_eq!(applied_count(&conn), 2);
        assert_eq!(max_version(&conn), Some(1));

        // And the table has both columns.
        conn.execute("INSERT INTO foo (id, name) VALUES (1, 'test')", [])
            .expect("insert");
    }

    // --- Already-migrated DB is a no-op ---

    #[rstest::rstest]
    fn already_migrated_db_is_noop() {
        // Given a database already at v1.
        let conn = test_conn();

        fn v0(conn: &Connection) -> Result<(), Report<MigrationError>> {
            conn.execute_batch("CREATE TABLE bar (id INTEGER PRIMARY KEY)")
                .change_context(MigrationError)?;
            Ok(())
        }
        fn v1(conn: &Connection) -> Result<(), Report<MigrationError>> {
            conn.execute_batch("ALTER TABLE bar ADD COLUMN val TEXT")
                .change_context(MigrationError)?;
            Ok(())
        }

        let migrations = vec![
            Migration { version: 0, name: "create_bar", up: v0 },
            Migration { version: 1, name: "add_val", up: v1 },
        ];

        // Apply all migrations.
        run_migrations(&conn, &migrations).expect("first run");
        assert_eq!(applied_count(&conn), 2);

        // When running migrations again.
        run_migrations(&conn, &migrations).expect("second run");

        // Then no new migrations are applied.
        assert_eq!(applied_count(&conn), 2);
        assert_eq!(max_version(&conn), Some(1));
    }

    // --- Partial migration applies only pending ---

    #[rstest::rstest]
    fn partial_migration_applies_only_pending() {
        // Given a database at v0.
        let conn = test_conn();

        fn v0(conn: &Connection) -> Result<(), Report<MigrationError>> {
            conn.execute_batch("CREATE TABLE baz (id INTEGER PRIMARY KEY)")
                .change_context(MigrationError)?;
            Ok(())
        }
        fn v1(conn: &Connection) -> Result<(), Report<MigrationError>> {
            conn.execute_batch("ALTER TABLE baz ADD COLUMN a TEXT")
                .change_context(MigrationError)?;
            Ok(())
        }
        fn v2(conn: &Connection) -> Result<(), Report<MigrationError>> {
            conn.execute_batch("ALTER TABLE baz ADD COLUMN b TEXT")
                .change_context(MigrationError)?;
            Ok(())
        }

        // Apply only v0.
        run_migrations(
            &conn,
            &[Migration { version: 0, name: "create_baz", up: v0 }],
        )
        .expect("apply v0");
        assert_eq!(applied_count(&conn), 1);
        assert_eq!(max_version(&conn), Some(0));

        // When running with all 3 migrations.
        let all = vec![
            Migration { version: 0, name: "create_baz", up: v0 },
            Migration { version: 1, name: "add_a", up: v1 },
            Migration { version: 2, name: "add_b", up: v2 },
        ];
        run_migrations(&conn, &all).expect("apply remaining");

        // Then only v1 and v2 are newly applied.
        assert_eq!(applied_count(&conn), 3);
        assert_eq!(max_version(&conn), Some(2));

        // And the table has all columns.
        conn.execute("INSERT INTO baz (id, a, b) VALUES (1, 'x', 'y')", [])
            .expect("insert with all columns");
    }

    // --- Failed migration does not record version ---

    #[rstest::rstest]
    fn migration_failure_does_not_record_version() {
        // Given a fresh database with a migration that will fail.
        let conn = test_conn();

        fn v0_ok(conn: &Connection) -> Result<(), Report<MigrationError>> {
            conn.execute_batch("CREATE TABLE qux (id INTEGER PRIMARY KEY)")
                .change_context(MigrationError)?;
            Ok(())
        }
        fn v1_fail(_conn: &Connection) -> Result<(), Report<MigrationError>> {
            Err(error_stack::Report::new(MigrationError).attach("intentional failure"))
        }

        let migrations = vec![
            Migration { version: 0, name: "create_qux", up: v0_ok },
            Migration { version: 1, name: "will_fail", up: v1_fail },
        ];

        // When running migrations and v1 fails.
        let result = run_migrations(&conn, &migrations);

        // Then an error is returned.
        assert!(result.is_err());

        // And v0 was applied.
        assert_eq!(max_version(&conn), Some(0));
        assert_eq!(applied_count(&conn), 1);

        // And the table from v0 exists.
        assert!(table_exists(&conn, "qux"));
    }
}
