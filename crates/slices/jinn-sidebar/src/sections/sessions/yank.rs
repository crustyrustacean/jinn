//! Session-ID yank — copying the highlighted session's UUID to the clipboard.

use jinn_kernel::common::app_state::AppState;
use jinn_kernel::protocol::IntentResult;

/// Why a session-ID yank can be rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionYankError {
    /// The sessions section is not focused.
    WrongSection,
    /// No session is highlighted.
    NoSelection,
}

/// Validates that a session-ID yank can proceed.
///
/// # Errors
///
/// Returns [`SessionYankError`] if the sessions section is not focused or no
/// session is highlighted.
pub fn validate_session_yank_id(state: &AppState) -> Result<(), SessionYankError> {
    if !matches!(
        state.frontend.sidebar_section(),
        Some(jinn_sidebar_msg::SidebarSectionId::Sessions)
    ) {
        return Err(SessionYankError::WrongSection);
    }
    let has_selection = state
        .frontend
        .with_sections(|s| s.sessions.selected_id.is_some(), || false);
    if !has_selection {
        return Err(SessionYankError::NoSelection);
    }
    Ok(())
}

/// Yanks the highlighted session's ID to the clipboard and arms the
/// "Session ID yanked" banner.
///
/// The clipboard write rides [`jinn_slices::TuiSignals::yank_text`]: the TUI
/// layer drains the signal after dispatch and hands the text to the system
/// clipboard, so this slice touches no clipboard backend. The banner is a
/// flag on `FrontendState` — the same transient confirmation the close prompt
/// uses — and the next keypress dismisses it before dispatch runs, so this
/// action only ever sets it.
pub fn handle_session_yank_id(state: &mut AppState) -> IntentResult {
    if validate_session_yank_id(state).is_err() {
        return IntentResult::empty();
    }
    // Re-read after validating rather than trusting the earlier read: the
    // cursor is a session, and if it went away between the two the yank is
    // simply refused. (See `handle_session_close_with_lifecycle` for the
    // same shape.)
    let Some(selected) = state
        .frontend
        .with_sections(|s| s.sessions.selected_id.clone(), || None)
    else {
        return IntentResult::empty();
    };
    state
        .frontend
        .update_scope(|s| s.signals.yank_text = Some(selected.to_string()));
    state.frontend.session_id_yank_popup = true;
    IntentResult::empty()
}
