//! Pinned context sidebar pane management.
//!
//! Contains the [`PaneFocus`] enum, the well-known [`CHAT_PANE`] area ID,
//! and methods on [`TuiApp`] for opening, closing, and toggling the pinned pane.

use ratatui_spatial_splits::AreaId;

use super::TuiApp;

/// Well-known area ID for the chat pane in the split layout.
pub(crate) const CHAT_PANE: AreaId = AreaId(1);

/// Which pane currently has keyboard focus in the Chat tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneFocus {
    /// The chat log pane (left side).
    Chat,
    /// The pinned context sidebar pane (right side).
    Pinned,
}

impl TuiApp {
    /// Opens the pinned context sidebar pane by splitting the chat area vertically.
    ///
    /// # Panics
    ///
    /// Panics if `CHAT_PANE` is not a valid leaf in the split manager.
    #[expect(
        clippy::expect_used,
        reason = "CHAT_PANE invariant maintained by split manager"
    )]
    pub fn open_pinned_pane(&mut self) {
        if self.pinned_pane_visible {
            // Already visible — just ensure focus is set.
            self.pane_focus = PaneFocus::Pinned;
            return;
        }
        // Defensive: if we have a stale tracked ID in the tree, reuse it.
        if let Some(id) = self.pinned_pane_id
            && self.split_manager.contains(id)
        {
            self.pinned_pane_visible = true;
            self.pane_focus = PaneFocus::Pinned;
            return;
        }
        let result = self
            .split_manager
            .split_vertical_with_ratio(CHAT_PANE, 0.7)
            .expect("CHAT_PANE should always be a valid leaf");
        self.pinned_pane_id = Some(result.new);
        self.pinned_pane_visible = true;
        self.pane_focus = PaneFocus::Pinned;
    }

    /// Closes the pinned context sidebar pane.
    pub fn close_pinned_pane(&mut self) {
        if !self.pinned_pane_visible {
            return;
        }
        if let Some(id) = self.pinned_pane_id {
            self.split_manager.close(id);
            self.pinned_pane_id = None;
        }
        self.pinned_pane_visible = false;
        self.pane_focus = PaneFocus::Chat;
    }

    /// Toggles the pinned context sidebar pane.
    pub fn toggle_pinned_pane(&mut self) {
        if self.pinned_pane_visible {
            self.close_pinned_pane();
        } else {
            self.open_pinned_pane();
        }
    }
}
