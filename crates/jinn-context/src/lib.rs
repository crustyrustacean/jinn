//! Portable context models and loaders.
//!
//! This crate owns prompt templates, attachment-path resolution, and
//! project-context files without depending on application state, actors, or
//! the domain kernel. The types it publishes can travel through crossing
//! contracts and be consumed by turn, lifecycle, and context-assembly slices.

#![forbid(unsafe_code)]

pub mod attachment_path;
pub mod env_context;
pub mod frontmatter;
mod prompt_template;

pub use env_context::ContextFile;
pub use prompt_template::{
    MAX_FUZZY_RESULTS, PromptTemplate, PromptTemplateParseError, PromptTemplateStore,
    PromptTemplateStoreError, expand_tokens, render_template_file,
};
