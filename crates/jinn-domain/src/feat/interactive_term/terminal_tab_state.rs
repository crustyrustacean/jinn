//! Terminal tab state — the frontend mirror of the active `interactive_term`
//! session.
//!
//! The [`InteractiveTermActor`](crate::feat::interactive_term::interactive_term_actor::InteractiveTermActor)
//! writes this mirror from `TermScreenUpdated` events (plain-text screen,
//! cursor, visibility) and `TermControlChanged`; the renderer only reads it.
//! Keystrokes in control mode are *not* applied here — they go to the pty and
//! round-trip back as screen updates, so this state is always the program's
//! own rendering, never jinn's echo.

/// Who currently holds control of the active terminal session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TermControlHolder {
    /// The agent may send input via the `interactive_term_send` tool.
    #[default]
    Agent,
    /// The user took over with `i`; agent input is refused until handback.
    User,
}

/// Frontend mirror of the active `interactive_term` session.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TerminalTabState {
    /// The session currently shown in the terminal tab, if any.
    pub session_id: Option<String>,
    /// Last rendered screen (plain text, newline-separated rows).
    pub screen: String,
    /// Cursor position (row, col) on the mirrored screen.
    pub cursor: (u16, u16),
    /// Whether the program hid the cursor (TUIs hide it while redrawing).
    pub cursor_hidden: bool,
    /// Who holds control of the session.
    pub control: TermControlHolder,
    /// Size of the terminal tab's content rect last reported as `(rows, cols)`;
    /// dedupes resize publications.
    pub layout_size: (u16, u16),
}

impl TerminalTabState {
    /// Replaces the mirrored screen and cursor from a screen-update event.
    pub fn apply_screen(
        &mut self,
        session_id: &str,
        screen: String,
        cursor: (u16, u16),
        cursor_hidden: bool,
    ) {
        self.session_id = Some(session_id.to_owned());
        self.screen = screen;
        self.cursor = cursor;
        self.cursor_hidden = cursor_hidden;
    }

    /// Sets who holds control.
    pub fn set_control(&mut self, holder: TermControlHolder) {
        self.control = holder;
    }

    /// Returns the visible screen text (empty when no session is mirrored).
    #[must_use]
    pub fn screen(&self) -> &str {
        &self.screen
    }

    /// Records the tab's layout size, returning `true` when it changed and
    /// a resize should be published.
    pub fn record_layout_size(&mut self, rows: u16, cols: u16) -> bool {
        let changed = self.layout_size != (rows, cols);
        if changed {
            self.layout_size = (rows, cols);
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]

    use super::*;

    #[rstest::rstest]
    fn apply_screen_replaces_mirror() {
        // Given an empty terminal tab state.
        let mut state = TerminalTabState::default();

        // When applying a screen update.
        state.apply_screen("term-1", "hello\nworld".to_owned(), (1, 3), true);

        // Then the mirror carries the session, screen, cursor, and visibility.
        assert_eq!(state.session_id.as_deref(), Some("term-1"));
        assert_eq!(state.screen(), "hello\nworld");
        assert_eq!(state.cursor, (1, 3));
        assert!(state.cursor_hidden);
    }

    #[rstest::rstest]
    fn set_control_flips_holder() {
        // Given a default (agent-controlled) terminal tab state.
        let mut state = TerminalTabState::default();

        // When the user takes control.
        state.set_control(TermControlHolder::User);

        // Then control reads User.
        assert_eq!(state.control, TermControlHolder::User);
    }

    #[rstest::rstest]
    fn record_layout_size_reports_change_once() {
        // Given a default terminal tab state (0, 0).
        let mut state = TerminalTabState::default();

        // When recording a new layout size.
        let first = state.record_layout_size(24, 100);

        // Then the first report signals change.
        assert!(first);

        // When recording the same size again.
        let second = state.record_layout_size(24, 100);

        // Then no change is signaled.
        assert!(!second);
    }
}
