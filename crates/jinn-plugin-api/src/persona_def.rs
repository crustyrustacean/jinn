//! Persona contributions — the wire shape of one persona definition.
//!
//! [`PersonaDef`] mirrors the on-disk persona format (`+++`-delimited TOML
//! frontmatter carrying `name` and `description`, with a markdown body)
//! flattened to plain fields. The guest plugin parses files; the host
//! translates each entry into the core persona at publish time. An entry
//! with an empty or whitespace-only `name` drops that one persona, never
//! the whole batch.

use serde::{Deserialize, Serialize};

/// One persona: name, optional description, and the body text that becomes
/// part of the system prompt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersonaDef {
    /// Unique persona name.
    pub name: String,
    /// Short description for the picker UI, if the source declared one.
    pub description: Option<String>,
    /// The persona body — the actual system prompt text.
    pub body: String,
}
