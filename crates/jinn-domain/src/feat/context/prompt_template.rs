//! Prompt template data model, loading, and lookup.
//!
//! Templates are markdown files with TOML frontmatter (delimited by `+++`).
//! This crate provides types for loading, storing, and searching templates
//! independently of the application bus or state.
//!
//! The [`PromptTemplate`] data struct itself lives in `jinn-domain` so it
//! can travel across the actor boundary. This crate owns the loading, parsing,
//! and storage logic.

mod attachment_path;
mod expand;
mod loader;
mod store;
#[cfg(test)]
mod store_tests;

pub use attachment_path::ScanResult;
pub use attachment_path::scan_at_paths;
pub use expand::expand_tokens;
pub use loader::PromptTemplateParseError;
pub use loader::render_template_file;
pub use store::PromptTemplateStore;
pub use store::PromptTemplateStoreError;
