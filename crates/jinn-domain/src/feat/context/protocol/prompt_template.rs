//! Prompt template domain types.
//!
//! A [`PromptTemplate`] is a reusable prompt loaded from `~/.config/jinn/prompts/`.
//! It consists of metadata (name, description) and a body. Templates are referenced
//! inline in the chat input via `$name` syntax.

use serde::{Deserialize, Serialize};

/// A reusable prompt template loaded from `~/.config/jinn/prompts/`.
///
/// Parsed from a markdown file with TOML frontmatter:
///
/// ```markdown
/// +++
/// name = "code-review"
/// description = "Perform a thorough code review"
/// +++
/// You are an expert code reviewer...
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTemplate {
    /// Unique identifier used in `$name` references.
    pub name: String,
    /// Short human-readable description shown in the autocomplete popup.
    pub description: String,
    /// The full template body text.
    pub body: String,
}
