//! Shared application constants and utilities.
//!
//! Foundational helpers used across multiple jinn crates:
//!
//! - **`app_info`** — application identity constants (`APP_NAME`, `PREFS_FILE_NAME`).
//! - **`app_paths`** — well-known filesystem paths (config dir, data dir, etc.).
//! - **`toml_patch`** — comment-preserving TOML document patcher for user-editable
//!   config files.
//! - **`process_isolation` / `process_kill`** — spawning children in their own
//!   process group, and terminating a child with its whole descendant tree.
//! - **`system_resource`** — loading a system resource file from a user-override
//!   directory with a system-default fallback.

pub mod app_info;
pub mod app_paths;
pub mod process_isolation;
pub mod process_kill;
pub mod system_resource;
pub mod template_check;
pub mod toml_patch;
