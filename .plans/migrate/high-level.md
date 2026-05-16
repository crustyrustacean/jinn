# Plan: Add Database Migration Module

## Problem

The application uses a single SQLite database (`sessions.db`) for session persistence. The schema is embedded as a `const SCHEMA` string in `sqlite.rs` and re-executed with `CREATE TABLE IF NOT EXISTS` on every `connect()`. There is no versioning — any future schema change requires manual migration with risk of data loss. There is no way to evolve the database safely over time.

## Proposed Solution

1. **Generic migration infrastructure** in `common/migration.rs` — a `Migration` struct (version, name, Rust function pointer), a `run_migrations()` function, and a `MigrationError` type. The runner creates a `schema_migrations` table if absent, reads the current version, and applies pending migrations in order. Each migration is a Rust function receiving `&Connection` so it can do arbitrary DDL/DML.

2. **Session-specific migrations** in `session_store/migration.rs` — defines the migration list for the session database. The current schema becomes migration v0 (baseline). The `SCHEMA` const is removed from `sqlite.rs`.

3. **Automatic execution** — `SqliteSessionStore::connect()` calls `run_migrations()` after pragmas, before returning the connection. This guarantees no code path can use an unmigrated database.

## Key Decisions

- **Migration function signature**: `fn(&Connection) -> Result<(), Report<MigrationError>>` — gives full access to `rusqlite::Connection` for arbitrary DDL/DML, not just SQL strings.
- **Migration ordering**: `Vec<Migration>` registered in order, each with a version number and name. `schema_migrations` table stores the current version.
- **Where to run**: Inside `SqliteSessionStore::connect()`, after pragmas, before returning the connection. This makes it impossible to use an unmigrated database. The user explicitly rejected running it as a separate startup step in `app.rs` in favor of this guarantee.
- **Connection model**: The session store uses a per-request `connect()` model — no connection pooling. Each async method creates a fresh `SqliteSessionStore` and calls `connect()` inside `spawn_blocking`. This means the migration check (`SELECT MAX(version) FROM schema_migrations`) runs on every save/load/delete/fork. The overhead is negligible (single-row table read) and the safety guarantee is worth it. Connection pooling is orthogonal to this task and not in scope.
- **Hybrid location**: Generic migration infrastructure lives in `common/migration.rs`. Session-specific migrations live in `session_store/migration.rs`. The common module can be reused for future databases.
- **Rust functions, not SQL strings**: Migrations are actual Rust functions so they can include data transformations, not just DDL.
- **Baseline approach**: The current `SCHEMA` const becomes migration v0. Existing databases will have this already applied (tables exist), so the migration system needs to handle the case where tables already exist (the `CREATE TABLE IF NOT EXISTS` in v0 handles this naturally).
- **Testing approach**: Delete the database and run from scratch to verify migrations apply correctly. All existing tests should pass unchanged since the resulting schema is identical.

## Acceptance Criteria

- A generic `run_migrations(conn, migrations)` function exists in `common/migration.rs`
- A session-specific migration list exists in `session_store/migration.rs` with v0 = current schema
- `SqliteSessionStore::connect()` automatically runs migrations on every connection
- The `SCHEMA` const is removed from `sqlite.rs` — schema creation is owned by the migration system
- All existing session store tests pass unchanged (same schema, just created via migration v0)
- Migration tests cover: fresh DB (applies all), already-migrated DB (no-op), partial migration (applies only pending)

## Implementation Phases

- [x] Phase 1: Generic migration infrastructure
  - [x] Create `nullslop-domain/src/common/migration.rs` with `MigrationError`, `Migration` struct, and `run_migrations()`
  - [x] Register module in `nullslop-domain/src/common.rs`

- [x] Phase 2: Session migration module + integrate into connect
  - [x] Create `nullslop-domain/src/feat/session/session_store/migration.rs` with v0 baseline and `session_migrations()`
  - [x] Register module in `session_store.rs`
  - [x] Modify `SqliteSessionStore::connect()` to call `run_migrations()` and remove `SCHEMA` const
  - [x] Verify all existing tests pass

- [x] Phase 3: Migration tests
  - [x] Fresh DB → applies all migrations
  - [x] Already-migrated DB → no-op
  - [x] Partial state → applies only pending
  - [x] Schema verification: migration v0 produces same tables as old SCHEMA const
