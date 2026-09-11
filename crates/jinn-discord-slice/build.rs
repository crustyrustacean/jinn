//! Build script: creates a SQLite DB with the post-migration schema and
//! exposes its path as `DAOW_DATABASE_URL` so the `#[dao]` macro can
//! validate the thread-map's SQL at compile time. Same schema-crate
//! pattern as the kernel's build script.

#![allow(warnings, reason = "want to fail fast")]

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

    println!("cargo:rerun-if-changed=../jinn-session-schema/src/lib.rs");
    println!("cargo:rerun-if-changed=../jinn-session-schema/src/migrate.rs");
    println!("cargo:rerun-if-changed=../jinn-session-schema/src/legacy.rs");
}
