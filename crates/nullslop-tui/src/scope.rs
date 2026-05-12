//! Keymap scopes for context-sensitive key handling.
//!
//! The scope determines which set of keybindings is active.

/// The current keymap context.
///
/// Controls which keybindings are active. Set via
/// [`ratatui_which_key::WhichKeyState::set_scope`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    /// Normal mode — navigation and commands.
    Normal,
    /// Dashboard mode — actor list navigation.
    Dashboard,
    /// Pinned panel mode — pinned entry navigation and management.
    Pinned,
    /// Picker mode — filtering and selecting a provider.
    Picker,
    /// Input mode — typing into the input buffer.
    Input,
    /// Tool content popup mode — scroll and close the popup.
    ToolContent,
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Normal => write!(f, "Normal"),
            Self::Dashboard => write!(f, "Dashboard"),
            Self::Pinned => write!(f, "Pinned"),
            Self::Picker => write!(f, "Picker"),
            Self::Input => write!(f, "Input"),
            Self::ToolContent => write!(f, "ToolContent"),
        }
    }
}

impl std::str::FromStr for Scope {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Normal" => Ok(Self::Normal),
            "Dashboard" => Ok(Self::Dashboard),
            "Pinned" => Ok(Self::Pinned),
            "Picker" => Ok(Self::Picker),
            "Input" => Ok(Self::Input),
            "ToolContent" => Ok(Self::ToolContent),
            _ => Err(()),
        }
    }
}
