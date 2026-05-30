//! Persona data model.

use std::path::PathBuf;

/// A parsed persona ready for use in the system prompt.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Persona {
    /// Unique persona name (from frontmatter).
    pub name: String,
    /// Short description for the picker UI.
    pub description: String,
    /// The persona body - the actual system prompt text.
    pub body: String,
    /// File path this persona was loaded from.
    pub file_path: PathBuf,
}
