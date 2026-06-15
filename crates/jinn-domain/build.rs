//! Build script: creates a SQLite DB with the post-v20 schema and exposes its
//! path as `DAO_DATABASE_URL` so the `#[dao]` macro can validate `#[query]` /
//! `#[execute]` SQL against a real database at compile time.
//!
//! Mirrors `dao`'s own `build.rs` pattern. The DB lives under `OUT_DIR`, so it
//! is regenerated per-build and never committed; `dao_schema.sql` is the
//! committed source of truth.

use std::path::PathBuf;

fn main() {
    let schema = include_str!("dao_schema.sql");

    let dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("dao_validation");
    std::fs::create_dir_all(&dir).expect("create dao_validation dir");

    let db_path = dir.join("validation.db");
    // Always recreate so the schema is current.
    let _ = std::fs::remove_file(&db_path);

    let conn = rusqlite::Connection::open(&db_path)
        .unwrap_or_else(|e| panic!("failed to open dao validation db: {e}"));
    conn.execute_batch(schema)
        .expect("failed to apply dao_schema.sql");

    println!(
        "cargo:rustc-env=DAO_DATABASE_URL={}",
        db_path.to_string_lossy()
    );
    println!("cargo:rerun-if-changed=dao_schema.sql");
}
