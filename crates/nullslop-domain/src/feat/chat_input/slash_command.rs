//! Slash command definitions and registry.
//!
//! Defines the set of slash commands available in the chat input box.
//! Each command has a name (e.g. `"new"`) and a short description for the
//! autocomplete popup. Commands are matched on submit — the buffer text is
//! checked against the registered names.

/// A slash command the user can invoke from the chat input box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashCommand {
    /// Create a new chat session.
    New,
}

/// A single entry for the slash command autocomplete popup.
#[derive(Debug, Clone)]
pub struct SlashCommandEntry {
    /// The command name without the leading `/` (e.g. `"new"`).
    pub name: String,
    /// Short human-readable description for the popup.
    pub description: String,
}

impl SlashCommand {
    /// Returns all registered slash commands as popup entries.
    pub fn all_entries() -> Vec<SlashCommandEntry> {
        vec![SlashCommandEntry {
            name: "new".to_owned(),
            description: "Create a new session".to_owned(),
        }]
    }

    /// Looks up a slash command by name (without the leading `/`).
    pub fn lookup(name: &str) -> Option<Self> {
        match name {
            "new" => Some(Self::New),
            _ => None,
        }
    }
}
