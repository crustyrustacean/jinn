//! Keymap search picker — search and execute keybindings.
//!
//! This component provides the data types for the keymap picker UI.
//! [`KeymapEntry`] holds a single fully-resolved leaf binding from the keymap.
//! The picker reuses the shared `Picker*` commands and `SelectionState<KeymapEntry>`.
//!
//! Tree-walking collection functions that build `KeymapEntry` lists from the
//! keymap live in `nullslop-tui` (they need the concrete key types).

pub mod entries;

pub use entries::KeymapEntry;
