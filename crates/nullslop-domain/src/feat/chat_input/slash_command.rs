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
    /// Summarize conversation history into a structured checkpoint.
    Compact,
    /// Compact all messages, ignoring the token reserve.
    CompactAll,
    /// Run a named workflow.
    Workflow,
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
        vec![
            SlashCommandEntry {
                name: "compact".to_owned(),
                description: "Summarize conversation history".to_owned(),
            },
            SlashCommandEntry {
                name: "compact-all".to_owned(),
                description: "Compact all messages (ignores reserve)".to_owned(),
            },
            SlashCommandEntry {
                name: "new".to_owned(),
                description: "Create a new session".to_owned(),
            },
            SlashCommandEntry {
                name: "workflow".to_owned(),
                description: "Run a named workflow".to_owned(),
            },
        ]
    }

    /// Looks up a slash command by name (without the leading `/`).
    pub fn lookup(name: &str) -> Option<Self> {
        match name {
            "compact" => Some(Self::Compact),
            "compact-all" => Some(Self::CompactAll),
            "new" => Some(Self::New),
            "workflow" => Some(Self::Workflow),
            _ => None,
        }
    }
}
