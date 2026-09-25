//! Compatibility exports for portable environment-context builders and loaders.
//!
//! The implementation is owned by `jinn-context`; this path preserves current
//! assembly, session-init, and kernel consumers during migration.

pub use jinn_context::env_context::{
    ContextFile, context_files_section, cwd_section, date_section, load_project_context_files,
    load_project_context_files_sync, persona_section,
};
