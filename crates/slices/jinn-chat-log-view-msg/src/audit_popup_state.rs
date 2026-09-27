//! The audit popup's visibility.
//!
//! The audit popup is a chat-log surface, so its visibility is a cell
//! this slice owns rather than a field on the kernel's
//! `FrontendState`. It stays a *global* toggle — one flag for the
//! process, not one per session — which is why it lives in its own cell
//! instead of alongside the per-session [`ChatLogViews`] map.

use jinn_slices::SlotKey;

/// Whether the audit popup is shown for the currently selected chat
/// entry.
///
/// Global (not per-session); not persisted across process restarts.
/// Default: hidden.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AuditPopupState {
    /// Whether the popup is currently shown.
    pub visible: bool,
}

impl AuditPopupState {
    /// Flips the popup's visibility and returns the new value.
    pub fn toggle(&mut self) -> bool {
        self.visible = !self.visible;
        self.visible
    }
}

/// The slot key the audit-popup cell lives under.
#[must_use]
pub fn audit_popup_slot() -> SlotKey {
    SlotKey::builtin("chat-log-view", "audit-popup")
}
