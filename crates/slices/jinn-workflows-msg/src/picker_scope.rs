//! The workflows picker's identity.
//!
//! The scope is the picker's public handle: a slice that wants to react to
//! the menu being open matches this id against the current focus scope, and
//! needs no other knowledge of the picker. It lives in the `-msg` crate so
//! any slice can ask "is the workflows picker open?" without depending on
//! the implementation crate.

use jinn_slices::SliceScopeId;
use jinn_slices::SlotKey;

/// The workflows picker's dynamic scope (input-capturing: the filter is
/// text).
#[must_use]
pub fn workflows_picker_scope() -> SliceScopeId {
    SliceScopeId::new("workflows", "picker")
}

/// The cell slot holding the workflows picker's state.
#[must_use]
pub fn workflows_picker_slot() -> SlotKey {
    SlotKey::builtin("workflows", "picker")
}

/// Result rows assumed before the render pass has measured the real popup.
///
/// Matches the other pickers' fallback, so the first keypress before any
/// render pages the same way everywhere.
pub const RESULTS_VIEWPORT_FALLBACK: usize = 20;
