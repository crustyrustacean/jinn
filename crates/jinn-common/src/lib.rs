//! Shared application constants and utilities.
//!
//! Foundational helpers used across multiple jinn crates:
//!
//! - **`app_info`** — application identity constants (`APP_NAME`, `PREFS_FILE_NAME`).
//! - **`app_paths`** — well-known filesystem paths (config dir, data dir, etc.).
//! - **`toml_patch`** — comment-preserving TOML document patcher for user-editable
//!   config files.

pub mod app_info;
pub mod app_paths;
pub mod process_isolation;
pub mod toml_patch;
