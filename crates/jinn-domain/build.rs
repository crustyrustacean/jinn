//! Build script: creates a SQLite DB with the post-v20 schema and exposes its
//! path as `DAOW_DATABASE_URL` so the `#[dao]` macro can validate `#[query]` /
//! `#[execute]` SQL against a real database at compile time.
//!
//! The schema is sourced from `jinn_session_schema::run_migrations` — the
//! single source of truth shared with the runtime migrator. No hand-written
//! `.sql` file to drift out of sync. See the `dao` crate's README ("The
//! schema-crate pattern") for the rationale.

use std::path::PathBuf;

fn main() {
    let dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("dao_validation");
    std::fs::create_dir_all(&dir).expect("create dao_validation dir");

    let db_path = dir.join("validation.db");
    // Always recreate so the schema is current.
    let _ = std::fs::remove_file(&db_path);

    let mut conn = rusqlite::Connection::open(&db_path)
        .unwrap_or_else(|e| panic!("failed to open dao validation db: {e}"));
    jinn_session_schema::run_migrations(&mut conn)
        .unwrap_or_else(|e| panic!("failed to apply migrations to dao validation db: {e}"));

    println!(
        "cargo:rustc-env=DAOW_DATABASE_URL={}",
        db_path.to_string_lossy()
    );

    // Rerun whenever the schema crate's source changes so new/edited migrations
    // appear in the validation DB on the next build.
    println!("cargo:rerun-if-changed=../jinn-session-schema/src/lib.rs");
    println!("cargo:rerun-if-changed=../jinn-session-schema/src/migrate.rs");
    println!("cargo:rerun-if-changed=../jinn-session-schema/src/legacy.rs");
}
