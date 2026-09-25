//! Reusable prompt-template value.
//!
//! Templates are markdown files with TOML frontmatter:
//!
//! ```markdown
//! +++
//! name = "code-review"
//! description = "Perform a thorough code review"
//! +++
//! You are an expert code reviewer...
//! ```

use serde::{Deserialize, Serialize};

mod expand;
mod loader;
mod store;
#[cfg(test)]
mod store_tests;

pub use expand::expand_tokens;
pub use loader::{PromptTemplateParseError, render_template_file};
pub use store::{MAX_FUZZY_RESULTS, PromptTemplateStore, PromptTemplateStoreError};

/// A reusable prompt template loaded from the user's or a project's prompt directory.
///
/// The serialized shape is part of the prompt-template compatibility surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTemplate {
    /// Unique identifier used in `#name` references.
    pub name: String,
    /// Short human-readable description shown in autocomplete.
    pub description: String,
    /// The full template body text.
    pub body: String,
}
