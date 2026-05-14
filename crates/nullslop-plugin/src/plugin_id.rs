//! Plugin identifier — derived from the plugin's directory name.

use std::fmt;

/// Unique identifier for a plugin.
///
/// Derived from the plugin's directory name (e.g., `turn-counter`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PluginId(String);

impl PluginId {
    /// Creates a new plugin ID from a directory name.
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self(name.to_owned())
    }

    /// Returns the plugin's name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
