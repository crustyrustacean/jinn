//! Standalone schema + migration crate for jinn's session store.
//!
//! This is the **single source of truth** for the SQLite schema. It is consumed
//! in two places, both of which must see the same schema:
//!
//! - **Build time:** `jinn-domain/build.rs` calls [`run_migrations`] on a fresh
//!   `OUT_DIR` database, then points `dao`'s `#[dao]` macro at it via
//!   `DAO_DATABASE_URL` for compile-time SQL validation.
//! - **Runtime:** jinn's session store calls [`run_migrations`] through a
//!   `dao::Pool::with_conn` closure on the user's `sessions.db` at startup.
//!
//! Living in its own crate (a leaf — no dependency on `jinn-domain`) is what
//! breaks the bootstrap cycle: `build.rs` can depend on this crate via
//! `[build-dependencies]`, whereas it cannot depend on the crate it belongs to.
//!
//! See the `dao` crate's README ("The schema-crate pattern") for the rationale.

mod legacy;
mod migrate;

pub use legacy::PersistableCoreV20;

use error_stack::{Report, ResultExt as _};
use wherror::Error;

/// Runs all pending schema migrations on the given connection.
///
/// The sequence runs with `PRAGMA foreign_keys=OFF`, then re-enables FK and
/// runs `PRAGMA foreign_key_check`. The FK toggle is essential: DDL such as
/// `DROP TABLE sessions` (used by the v15/v16/v20 table-rebuild migrations)
/// performs an implicit `DELETE` of all rows, which would otherwise fire the
/// application-level `ON DELETE CASCADE` and wipe every `session_history` and
/// `token_ledger` row. After the migrations complete (or fail), FK is
/// re-enabled and `foreign_key_check` verifies referential integrity.
///
/// Safe to call on an empty database (bootstraps the tracking table) and
/// idempotent on a fully-migrated one (each migration is guarded by a version
/// check).
///
/// # Errors
///
/// Returns an error if any migration fails, if the FK pragma cannot be toggled,
/// or if `foreign_key_check` reports integrity violations after the run.
pub fn run_migrations(conn: &mut rusqlite::Connection) -> Result<(), Report<SchemaMigrationError>> {
    conn.pragma_update(None, "foreign_keys", "OFF")
        .change_context(SchemaMigrationError)
        .attach("disable foreign_keys before migration")?;

    let migrate_result = migrate::run_pending(conn);

    // Always re-enable FK + check integrity, even if a migration failed, so the
    // connection is left in its normal (FK-on) state.
    conn.pragma_update(None, "foreign_keys", "ON")
        .change_context(SchemaMigrationError)
        .attach("re-enable foreign_keys after migration")?;

    let violations = fk_violations(conn)?;
    match (migrate_result, violations) {
        (Ok(()), empty) if empty.is_empty() => Ok(()),
        (Ok(()), tables) => Err(Report::new(SchemaMigrationError)
            .attach("foreign_key_check reported violations after migration")
            .attach(format!("violating tables: {}", tables.join(", ")))),
        (Err(e), _) => Err(e),
    }
}

/// Returns the names of tables that violate foreign-key constraints.
///
/// `PRAGMA foreign_key_check` returns one row per violation (empty = clean).
/// Columns: `table`, `rowid`, `parent`, `fkid` — only `table` is bound.
fn fk_violations(conn: &mut rusqlite::Connection) -> Result<Vec<String>, Report<SchemaMigrationError>> {
    let mut stmt = conn
        .prepare("PRAGMA foreign_key_check")
        .change_context(SchemaMigrationError)
        .attach("prepare foreign_key_check")?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .change_context(SchemaMigrationError)
        .attach("query foreign_key_check")?;
    let mut tables = Vec::new();
    for row in rows {
        tables.push(
            row.change_context(SchemaMigrationError)
                .attach("collect foreign_key_check row")?,
        );
    }
    Ok(tables)
}

/// Error type for schema migration failures.
///
/// Carries no variants — the failure detail lives in the `error_stack::Report`
/// context attachments (which migration failed, what SQL, which table violated
/// a constraint). This mirrors jinn's `SessionStoreError` convention.
#[derive(Debug, Error)]
#[error(debug)]
pub struct SchemaMigrationError;
