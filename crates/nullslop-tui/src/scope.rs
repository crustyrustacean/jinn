//! Keymap scopes for context-sensitive key handling.
//!
//! The scope determines which set of keybindings is active.
//! Each sidebar section has its own scope so section-specific keys
//! (like `r` for rename vs pin-relative) are unambiguous.

/// The current keymap context.
///
/// Controls which keybindings are active. Set via
/// [`ratatui_which_key::WhichKeyState::set_scope`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    /// Normal mode — navigation and commands.
    Normal,
    /// Sidebar — Persona section.
    SidebarPersona,
    /// Sidebar — Pins section.
    SidebarPins,
    /// Sidebar — Sessions section.
    SidebarSessions,
    /// Picker mode — filtering and selecting a provider.
    Picker,
    /// Input mode — typing into the input buffer.
    Input,
    /// Arg input mode — typing positional args for a lifecycle command.
    ArgInput,
    /// Token budget input mode — typing a numeric budget value.
    TokenBudgetInput,
    /// Rename session input mode — editing a session title.
    RenameSessionInput,
    /// Sidebar resize mode — adjusting sidebar width.
    SidebarResize,
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Normal => write!(f, "Normal"),
            Self::SidebarPersona => write!(f, "SidebarPersona"),
            Self::SidebarPins => write!(f, "SidebarPins"),
            Self::SidebarSessions => write!(f, "SidebarSessions"),
            Self::Picker => write!(f, "Picker"),
            Self::Input => write!(f, "Input"),
            Self::ArgInput => write!(f, "ArgInput"),
            Self::TokenBudgetInput => write!(f, "TokenBudgetInput"),
            Self::RenameSessionInput => write!(f, "RenameSessionInput"),
            Self::SidebarResize => write!(f, "SidebarResize"),
        }
    }
}

impl std::str::FromStr for Scope {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Normal" => Ok(Self::Normal),
            "SidebarPersona" => Ok(Self::SidebarPersona),
            "SidebarPins" => Ok(Self::SidebarPins),
            "SidebarSessions" => Ok(Self::SidebarSessions),
            "Picker" => Ok(Self::Picker),
            "Input" => Ok(Self::Input),
            "ArgInput" => Ok(Self::ArgInput),
            "TokenBudgetInput" => Ok(Self::TokenBudgetInput),
            "RenameSessionInput" => Ok(Self::RenameSessionInput),
            "SidebarResize" => Ok(Self::SidebarResize),
            _ => Err(()),
        }
    }
}
