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
    /// Sidebar — Minimap section (display-only).
    SidebarMinimap,
    /// Picker — Provider/model selection.
    PickerProvider,
    /// Picker — Keymap search.
    PickerKeymap,
    /// Picker — Session browser.
    PickerSession,
    /// Picker — Persona selection.
    PickerPersona,
    /// Picker — Theme selection.
    PickerTheme,
    /// Picker — Session fork point selection.
    PickerFork,
    /// Picker — Session lifecycle recipe selection.
    PickerLifecycle,
    /// Input mode — typing into the input buffer.
    Input,
    /// Arg input mode — typing positional args for a lifecycle command.
    ArgInput,
    /// Token budget input mode — typing a numeric budget value.
    TokenBudgetInput,
    /// Sliding window input mode — typing a numeric window size.
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
            Self::SidebarMinimap => write!(f, "SidebarMinimap"),
            Self::PickerProvider => write!(f, "Picker(provider)"),
            Self::PickerKeymap => write!(f, "Picker(keymap)"),
            Self::PickerSession => write!(f, "Picker(session)"),
            Self::PickerPersona => write!(f, "Picker(persona)"),
            Self::PickerTheme => write!(f, "Picker(theme)"),
            Self::PickerFork => write!(f, "Picker(fork)"),
            Self::PickerLifecycle => write!(f, "Picker(lifecycle)"),
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
            "SidebarMinimap" => Ok(Self::SidebarMinimap),
            "Picker(provider)" => Ok(Self::PickerProvider),
            "Picker(keymap)" => Ok(Self::PickerKeymap),
            "Picker(session)" => Ok(Self::PickerSession),
            "Picker(persona)" => Ok(Self::PickerPersona),
            "Picker(theme)" => Ok(Self::PickerTheme),
            "Picker(fork)" => Ok(Self::PickerFork),
            "Picker(lifecycle)" => Ok(Self::PickerLifecycle),
            "Input" => Ok(Self::Input),
            "ArgInput" => Ok(Self::ArgInput),
            "TokenBudgetInput" => Ok(Self::TokenBudgetInput),
            "RenameSessionInput" => Ok(Self::RenameSessionInput),
            "SidebarResize" => Ok(Self::SidebarResize),
            _ => Err(()),
        }
    }
}
