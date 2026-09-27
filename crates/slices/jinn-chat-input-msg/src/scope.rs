//! The chat input's own scopes.
//!
//! The chat input box is a scope-owning surface, not a static kernel mode:
//! its keys resolve in a dynamic slice scope, so the kernel holds no
//! `Scope::ChatInput` variant and the TUI layer never learns this box
//! exists.

use jinn_slices::SliceScopeId;

/// The scope the chat input box's keys live in.
///
/// Built with [`SliceScopeId::new`], so it `captures_input`: the keymap
/// binds the box's editing keys and its printable-character catch-all
/// here rather than in a static kernel scope.
#[must_use]
pub fn chat_input_scope() -> SliceScopeId {
    SliceScopeId::new("chat-input", "view")
}
