//! Scroll methods for [`ChatSessionState`](super::ChatSessionState).

use std::sync::atomic::Ordering;

use super::ChatSessionState;

impl ChatSessionState {
    /// The current scroll offset (lines to skip from top).
    ///
    /// Returns `None` when auto-scrolled to the bottom, or `Some(n)` when
    /// the user has manually scrolled to a specific offset.
    pub fn scroll_offset(&self) -> Option<u16> {
        self.ui.scroll_offset
    }

    /// Whether the conversation is scrolled to the bottom (auto-scroll position).
    pub fn is_at_bottom(&self) -> bool {
        self.ui.scroll_offset.is_none()
    }

    /// Scroll up (toward older messages) by the given number of lines.
    ///
    /// If currently at the bottom (auto-scroll), resolves to `last_max_offset` first
    /// so the scroll is relative to the actual bottom position.
    pub fn scroll_up(&mut self, amount: u16) {
        let current = self
            .ui
            .scroll_offset
            .unwrap_or(self.ui.last_max_offset.load(Ordering::Relaxed));
        self.ui.scroll_offset = Some(current.saturating_sub(amount));
    }

    /// Scroll down (toward newer messages) by the given number of lines.
    ///
    /// If the resulting offset reaches or exceeds `last_max_offset`, resets to
    /// auto-scroll (bottom).
    pub fn scroll_down(&mut self, amount: u16) {
        let current = self
            .ui
            .scroll_offset
            .unwrap_or(self.ui.last_max_offset.load(Ordering::Relaxed));
        let next = current.saturating_add(amount);
        if next >= self.ui.last_max_offset.load(Ordering::Relaxed) {
            self.ui.scroll_offset = None;
        } else {
            self.ui.scroll_offset = Some(next);
        }
    }

    /// Reset scroll to show the bottom of the conversation.
    pub fn reset_scroll(&mut self) {
        self.ui.scroll_offset = None;
    }

    /// Scroll to the very top of the conversation.
    pub fn scroll_to_top(&mut self) {
        self.ui.scroll_offset = Some(0);
    }

    /// Scroll to the very bottom of the conversation (auto-scroll).
    pub fn scroll_to_bottom(&mut self) {
        self.ui.scroll_offset = None;
    }

    /// Update the cached maximum scroll offset from the renderer.
    ///
    /// Called by the chat log element during each render so that
    /// scroll handlers can resolve the "at bottom" state into a concrete offset.
    pub fn set_last_max_offset(&self, max_offset: u16) {
        self.ui.last_max_offset.store(max_offset, Ordering::Relaxed);
    }
}
