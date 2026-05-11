//! Direction for tab cycling.

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
