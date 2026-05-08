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
    /// Workflow mode — workflow step list navigation.
    Workflow,
    /// Pinned panel mode — pinned entry navigation and management.
    Pinned,
    /// Picker mode — filtering and selecting a provider.
    Picker,
    /// Input mode — typing into the input buffer.
    Input,
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Normal => write!(f, "Normal"),
            Self::Dashboard => write!(f, "Dashboard"),
            Self::Workflow => write!(f, "Workflow"),
            Self::Pinned => write!(f, "Pinned"),
            Self::Picker => write!(f, "Picker"),
            Self::Input => write!(f, "Input"),
        }
    }
}

impl std::str::FromStr for Scope {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Normal" => Ok(Self::Normal),
            "Dashboard" => Ok(Self::Dashboard),
            "Workflow" => Ok(Self::Workflow),
            "Pinned" => Ok(Self::Pinned),
            "Picker" => Ok(Self::Picker),
            "Input" => Ok(Self::Input),
            _ => Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_is_less_than_input() {
        // Given the two scopes.
        // When comparing.
        // Then Normal < Input.
        assert!(Scope::Normal < Scope::Input);
    }

    #[test]
    fn picker_is_between_normal_and_input() {
        // Given the six scopes.
        // When comparing.
        // Then Normal < Dashboard < Workflow < Pinned < Picker < Input (derived from declaration order).
        assert!(Scope::Normal < Scope::Dashboard);
        assert!(Scope::Dashboard < Scope::Workflow);
        assert!(Scope::Workflow < Scope::Pinned);
        assert!(Scope::Pinned < Scope::Picker);
        assert!(Scope::Picker < Scope::Input);
    }
}
