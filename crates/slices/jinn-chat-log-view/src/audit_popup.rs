//! The audit popup's visibility — the one piece of chat-log state that
//! is not per-session.
//!
//! The popup's text and geometry are in [`crate::chat_log::audit_popup`];
//! this module owns only the flag, so the kernel's `FrontendState` no
//! longer carries chat-log state and the TUI adapter reads a cell rather
//! than a frontend field.

use jinn_chat_log_view_msg::{AuditPopupState, audit_popup_slot};
use jinn_slices::Slices;

/// Flips the popup's visibility and returns the new value.
///
/// Returns [`None`] when the cell is unregistered — the popup then
/// renders nothing, which is the same observable outcome as hidden.
#[must_use]
pub fn toggle(slices: &Slices) -> Option<bool> {
    let cell = slices.reader::<AuditPopupState>(&audit_popup_slot())?;
    Some(cell.update(AuditPopupState::toggle))
}

/// Whether the popup is currently shown.
///
/// An unregistered cell reads as hidden.
#[must_use]
pub fn is_visible(slices: &Slices) -> bool {
    slices
        .reader::<AuditPopupState>(&audit_popup_slot())
        .is_some_and(|cell| cell.read().visible)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]

    use super::*;

    #[rstest::rstest]
    fn toggle_flips_the_cell_visibility() {
        // Given a registry with the audit-popup cell registered.
        let slices = Slices::new();
        slices
            .register(audit_popup_slot(), AuditPopupState::default())
            .expect("the cell is registered once for this test");

        // When toggling once.
        let after_first = toggle(&slices);

        // Then the popup becomes visible.
        assert_eq!(after_first, Some(true));
        assert!(is_visible(&slices));
    }

    #[rstest::rstest]
    fn toggling_twice_returns_to_hidden() {
        // Given a registry with the popup already toggled on.
        let slices = Slices::new();
        slices
            .register(audit_popup_slot(), AuditPopupState { visible: true })
            .expect("the cell is registered once for this test");

        // When toggling.
        let after = toggle(&slices);

        // Then the popup hides again.
        assert_eq!(after, Some(false));
        assert!(!is_visible(&slices));
    }

    #[rstest::rstest]
    fn unregistered_cell_reads_as_hidden() {
        // Given a registry with no cells at all.
        let slices = Slices::new();

        // When reading visibility.
        let visible = is_visible(&slices);

        // Then the popup is hidden rather than panicking.
        assert!(!visible);
    }
}
