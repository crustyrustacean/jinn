//! Tab domain: tab management types, active tab state, and tab navigation commands.

mod active_tab;
mod command;

pub use active_tab::ActiveTab;
pub use command::SwitchTab;
use serde::{Deserialize, Serialize};

/// Direction for tab cycling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TabDirection {
    /// Move to the next tab (wrapping).
    Next,
    /// Move to the previous tab (wrapping).
    Prev,
}

impl std::fmt::Display for TabDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Next => write!(f, "next"),
            Self::Prev => write!(f, "prev"),
        }
    }
}
