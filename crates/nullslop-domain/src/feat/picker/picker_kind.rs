//! Picker kind — identifies which picker is currently active.
//!
//! A single set of `Picker*` commands, `Mode::Picker`, `Scope::Picker`,
//! and keymap bindings serve all pickers. [`PickerKind`] determines which
//! `SelectionState` (from `nullslop-selection-widget`) the commands
//! operate on.

use serde::{Deserialize, Serialize};

/// Which picker is currently active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PickerKind {
    /// Provider/model picker.
    Provider,
    /// Context assembly strategy picker.
    ContextAssembly,
    /// Keymap search picker.
    Keymap,
    /// Session browser picker.
    Session,
}

impl std::fmt::Display for PickerKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Provider => write!(f, "models"),
            Self::ContextAssembly => write!(f, "context-assembly"),
            Self::Keymap => write!(f, "keybinds"),
            Self::Session => write!(f, "sessions"),
        }
    }
}
