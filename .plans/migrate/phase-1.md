# Phase 1: Generic Migration Infrastructure

## Problem

There is no way to evolve the SQLite database schema over time. We need a generic migration runner that can be reused for any database. This phase creates the infrastructure; Phase 2 wires it into the session store.

## What Moves / What Stays

**New:**
- `nullslop-domain/src/common/migration.rs` — `MigrationError`, `Migration` struct, `run_migrations()` function

**Modified:**
- `nullslop-domain/src/common.rs` — add `pub mod migration;`

**Unchanged:**
- Everything else. Session store, `sqlite.rs`, `session_store.rs` are all untouched this phase.

## File Changes

### 1. Create `nullslop-domain/src/common/migration.rs`

```rust
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
            version  INTEGER PRIMARY KEY,
            name     TEXT NOT NULL,
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
```

### 2. Modify `nullslop-domain/src/common.rs`

Add `pub mod migration;` to the module list.

## Implementation Order

1. Create `migration.rs`
2. Register module in `common.rs`
3. Run `just check`

## Acceptance Criteria

- [x] `nullslop-domain/src/common/migration.rs` exists with `MigrationError`, `Migration`, and `run_migrations()`
- [x] `common.rs` includes `pub mod migration;`
- [x] `just check` passes

---

## Review: Phase 1 — Generic Migration Infrastructure

### Changes

Created `common/migration.rs` with `MigrationError`, `Migration` struct, and `run_migrations()` function. Registered the module in `common.rs`.

### Divergence Summary

Could not commit — fossil repo is read-only. All changes are in the working tree.

### Verification

`just check` passes.

### Risks

None. The module is self-contained and not yet consumed by anything.

### Next Steps

Phase 2: create session-specific migration module and wire into `SqliteSessionStore::connect()`.
