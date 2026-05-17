//! [`SessionsSection`] — the open sessions sidebar section.
//!
//! Implements [`SidebarSection`] for listing all sessions currently loaded
//! into memory. The active session (currently displayed) is highlighted with
//! a `▸` prefix. Navigating with j/k immediately switches the active session.

use crate::common::app_state::AppState;
use crate::feat::ui::sidebar::section_trait::{
    EnterFrom, SectionNavResult, SidebarIntent, SidebarSection, SidebarSectionId,
};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use throbber_widgets_tui::ThrobberState;

/// Active session indicator prefix.
const ACTIVE_PREFIX: &str = "▸ ";
/// Inactive session prefix (two spaces to align with `ACTIVE_PREFIX`).
const INACTIVE_PREFIX: &str = "  ";

/// Sessions section cursor state — stored on `FrontendState`.
///
/// Tracks the selected index within the sorted open sessions list.
/// `None` means no cursor (section not focused).
#[derive(Debug, Clone, Default)]
pub struct SessionsSectionState {
    /// Index into the sorted open sessions list.
    pub selected_index: Option<usize>,
}

/// A sorted snapshot of open session metadata for rendering.
struct SessionEntry {
    id: crate::protocol::SessionId,
    title: String,
    is_active: bool,
    created_at: jiff::Timestamp,
    is_idle: bool,
}

/// Collects all open sessions sorted by `created_at` descending (newest first).
fn sorted_open_sessions(state: &AppState) -> Vec<SessionEntry> {
    let active_id = &state.session.active_session;
    let mut entries: Vec<SessionEntry> = state
        .session
        .sessions
        .iter()
        .map(|(id, session)| SessionEntry {
            id: id.clone(),
            title: session.title().unwrap_or("Untitled Session").to_owned(),
            is_active: id == active_id,
            created_at: session.created_at().clone(),
            is_idle: session.is_idle(),
        })
        .collect();
    entries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    entries
}

/// Navigate within the sessions section.
///
/// Moves the cursor within the sessions list and immediately switches
/// the active session. Returns `Exhausted` when at a boundary or when
/// the list is empty.
pub fn navigate(intent: &SidebarIntent, state: &mut AppState) -> SectionNavResult {
    let sessions = sorted_open_sessions(state);
    if sessions.is_empty() {
        return SectionNavResult::Exhausted;
    }

    match intent {
        SidebarIntent::MoveDown => {
            let current = state.frontend.sessions_section.selected_index.unwrap_or(0);
            if current >= sessions.len() - 1 {
                return SectionNavResult::Exhausted;
            }
            let new_index = current + 1;
            state.frontend.sessions_section.selected_index = Some(new_index);
            SectionNavResult::Moved
        }
        SidebarIntent::MoveUp => {
            let current = state.frontend.sessions_section.selected_index.unwrap_or(0);
            if current == 0 {
                return SectionNavResult::Exhausted;
            }
            let new_index = current - 1;
            state.frontend.sessions_section.selected_index = Some(new_index);
            SectionNavResult::Moved
        }
        SidebarIntent::Action(_) => SectionNavResult::Moved,
    }
}

/// Place the cursor on this section from a given direction.
///
/// Finds the active session in the sorted list and positions the cursor there.
pub fn receive_cursor(state: &mut AppState, _enter_from: EnterFrom) {
    let sessions = sorted_open_sessions(state);
    let active_index = sessions.iter().position(|s| s.is_active).unwrap_or(0);
    state.frontend.sessions_section.selected_index = Some(active_index);
}

/// Activates the session under the cursor.
///
/// Called when the user presses Enter in the sessions section.
/// Switches `active_session` to the session at the cursor position.
pub fn handle_session_activate(state: &mut AppState) {
    use crate::common::app_state::FocusScope;
    use crate::feat::ui::sidebar::section_trait::SidebarSectionId;

    if state.frontend.sidebar.focused_section != SidebarSectionId::Sessions {
        return;
    }
    let Some(index) = state.frontend.sessions_section.selected_index else {
        return;
    };
    let sessions = sorted_open_sessions(state);
    let Some(entry) = sessions.get(index) else {
        return;
    };
    state.session.active_session = entry.id.clone();
    // Switch to insert mode so the user can start typing immediately.
    state.frontend.scope_stack.push(FocusScope::Input);
}

/// The open sessions sidebar section.
///
/// Renders all sessions loaded into memory with the active session highlighted.
#[derive(Debug)]
pub struct SessionsSection {
    /// Animation state for the working indicator.
    throbber_state: ThrobberState,
}

impl Default for SessionsSection {
    fn default() -> Self {
        Self {
            throbber_state: ThrobberState::default(),
        }
    }
}

impl SessionsSection {
    /// Creates a new sessions section.
    pub fn new() -> Self {
        Self::default()
    }
}

impl SidebarSection for SessionsSection {
    fn id(&self) -> SidebarSectionId {
        SidebarSectionId::Sessions
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, state: &AppState) {
        let sessions = sorted_open_sessions(state);
        let theme = &state.frontend.theme;
        let sidebar_focused = state.frontend.scope_stack.is_sidebar();
        let section_focused =
            sidebar_focused && state.frontend.sidebar.focused_section == SidebarSectionId::Sessions;

        let selected_index = state.frontend.sessions_section.selected_index;

        let mut lines = Vec::new();

        // Header.
        lines.push(Line::from(vec![Span::styled(
            " Sessions",
            Style::default()
                .fg(theme.primary_text)
                .add_modifier(Modifier::BOLD),
        )]));

        // Blank separator.
        lines.push(Line::from(""));

        if sessions.is_empty() {
            lines.push(Line::from(vec![Span::styled(
                " No open sessions",
                Style::default().fg(theme.muted_text),
            )]));
        } else {
            for (i, entry) in sessions.iter().enumerate() {
                let is_selected = section_focused && selected_index == Some(i);

                // Indicator: animated throbber when working, blank space when idle.
                let indicator_span = if entry.is_idle {
                    Span::raw(" ")
                } else {
                    let set = throbber_widgets_tui::symbols::throbber::BRAILLE_EIGHT;
                    let mut idx = self.throbber_state.index();
                    let len = set.symbols.len() as i8;
                    idx %= len;
                    if idx < 0 {
                        idx += len;
                    }
                    let ch = set.symbols[idx as usize];
                    Span::styled(ch.to_string(), Style::default().fg(Color::Cyan))
                };

                // Arrow: active session indicator.
                let arrow_span = if entry.is_active {
                    Span::styled(
                        ACTIVE_PREFIX.to_owned(),
                        Style::default().fg(theme.primary_text),
                    )
                } else {
                    Span::styled(INACTIVE_PREFIX.to_owned(), Style::default())
                };

                let title_style = if is_selected {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else if entry.is_active {
                    Style::default().fg(theme.primary_text)
                } else {
                    Style::default().fg(theme.muted_text)
                };

                // Truncate title to fit sidebar width (indicator(1) + prefix(2) + 1 padding).
                let max_title_len = area.width.saturating_sub(5) as usize;
                let truncated = truncate_str(&entry.title, max_title_len);

                lines.push(Line::from(vec![
                    indicator_span,
                    Span::raw(" "),
                    arrow_span,
                    Span::styled(truncated, title_style),
                ]));
            }

            // Advance animation for next frame.
            self.throbber_state.calc_next();
        }

        // Trailing gap.
        lines.push(Line::from(""));

        let widget = Paragraph::new(lines).block(Block::default().borders(Borders::NONE));
        frame.render_widget(widget, area);
    }

    fn content_height(&self, state: &AppState) -> u16 {
        let session_count = state.session.sessions.len() as u16;
        // header(1) + blank(1) + sessions(N) + trailing gap(1)
        3 + session_count.max(1) // max(1) for "No open sessions" message
    }
}

/// Truncates a string to fit within `max_len` graphemes, appending `…` if truncated.
fn truncate_str(s: &str, max_len: usize) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    if max_len == 0 {
        return String::new();
    }
    let graphemes: Vec<&str> = s.graphemes(true).collect();
    if graphemes.len() <= max_len {
        return s.to_owned();
    }
    let mut result: String = graphemes[..max_len.saturating_sub(1)].concat();
    result.push('…');
    result
}

// ---------------------------------------------------------------------------
// Close session handler
// ---------------------------------------------------------------------------

/// Why a session close can be rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionCloseError {
    /// The sessions section is not focused.
    WrongSection,
    /// No session is selected.
    NoSelection,
    /// The selected session is streaming or sending.
    SessionBusy,
}

/// Validates that a session close can proceed.
pub fn validate_session_close(state: &AppState) -> Result<(), SessionCloseError> {
    use crate::feat::ui::sidebar::section_trait::SidebarSectionId;

    // Sessions section must be focused.
    if state.frontend.sidebar.focused_section != SidebarSectionId::Sessions {
        return Err(SessionCloseError::WrongSection);
    }

    // A session must be selected.
    let index = state
        .frontend
        .sessions_section
        .selected_index
        .ok_or(SessionCloseError::NoSelection)?;

    // The selected session must be idle (not streaming/sending).
    let sessions = sorted_open_sessions(state);
    let entry = sessions.get(index).ok_or(SessionCloseError::NoSelection)?;
    let session = state
        .session
        .sessions
        .get(&entry.id)
        .ok_or(SessionCloseError::NoSelection)?;
    if !session.is_idle() {
        return Err(SessionCloseError::SessionBusy);
    }

    Ok(())
}

/// Handles `SidebarSessionClose` — closes the selected session.
///
/// Removes the session from the in-memory HashMap (keeps it in SQLite).
/// Activates the next session in the sorted list, clamping the index.
/// If the last session is closed, creates a new empty session.
pub fn handle_session_close(state: &mut AppState) -> crate::protocol::IntentResult {
    // Validate.
    if validate_session_close(state).is_err() {
        return crate::protocol::IntentResult::empty();
    }

    let index = state.frontend.sessions_section.selected_index.unwrap();
    let sessions = sorted_open_sessions(state);
    let closing_id = sessions[index].id.clone();

    // Remove from HashMap (keeps in SQLite).
    state.session.sessions.remove(&closing_id);

    if state.session.sessions.is_empty() {
        // Last session — create a new one (same logic as SessionNew intent).
        let new_session = crate::feat::session::chat_session::ChatSessionState::new();
        let new_id = new_session.session_id().clone();
        state.session.sessions.insert(new_id.clone(), new_session);
        state.session.active_session = new_id;
        state.frontend.sessions_section.selected_index = Some(0);
    } else {
        // Activate next session. Clamp index to valid range.
        let remaining = sorted_open_sessions(state);
        let clamped = index.min(remaining.len() - 1);
        state.session.active_session = remaining[clamped].id.clone();
        state.frontend.sessions_section.selected_index = Some(clamped);
    }

    crate::protocol::IntentResult::empty()
}

#[cfg(test)]
mod tests {
    use super::{SessionsSection, navigate, receive_cursor, sorted_open_sessions};
    use crate::common::app_state::AppState;
    use crate::feat::ui::sidebar::section_trait::{
        EnterFrom, SectionNavResult, SidebarIntent, SidebarSection, SidebarSectionId,
    };
    use crate::protocol::ChatEntry;

    // Helper: create state with N sessions.
    fn state_with_sessions(count: usize) -> AppState {
        let mut state = AppState::default();
        // Default state already has 1 session. Add more as needed.
        for i in 1..count {
            let session = crate::feat::session::chat_session::ChatSessionState::new();
            let id = session.session_id().clone();
            // Give each additional session a title.
            state.session.sessions.insert(id, {
                let mut s = crate::feat::session::chat_session::ChatSessionState::new();
                s.push_entry(ChatEntry::user(format!("message for session {i}")));
                s
            });
        }
        state
    }

    // --- Section identity ---

    #[rstest::rstest]
    fn section_id_is_sessions() {
        let section = SessionsSection::new();
        assert_eq!(section.id(), SidebarSectionId::Sessions);
    }

    // --- Content height ---

    #[rstest::rstest]
    fn content_height_with_one_session() {
        let section = SessionsSection::new();
        let state = AppState::default();
        assert_eq!(section.content_height(&state), 4); // header + blank + 1 session + gap
    }

    #[rstest::rstest]
    fn content_height_with_three_sessions() {
        let section = SessionsSection::new();
        let state = state_with_sessions(3);
        assert_eq!(section.content_height(&state), 6); // header + blank + 3 sessions + gap
    }

    // --- Navigation ---

    #[rstest::rstest]
    fn navigate_down_moves_cursor_without_switching() {
        // Given state with 3 sessions, cursor at index 0.
        let mut state = state_with_sessions(3);
        let original_active = state.session.active_session.clone();
        state.frontend.sessions_section.selected_index = Some(0);

        // When navigating down.
        let result = navigate(&SidebarIntent::MoveDown, &mut state);

        // Then the result is Moved.
        assert_eq!(result, SectionNavResult::Moved);
        // And the cursor moved to index 1.
        assert_eq!(state.frontend.sessions_section.selected_index, Some(1));
        // And the active session did NOT change.
        assert_eq!(state.session.active_session, original_active);
    }

    #[rstest::rstest]
    fn navigate_up_moves_cursor_without_switching() {
        // Given state with 3 sessions, cursor at index 2.
        let mut state = state_with_sessions(3);
        let sessions = sorted_open_sessions(&state);
        state.session.active_session = sessions[2].id.clone();
        state.frontend.sessions_section.selected_index = Some(2);
        let original_active = state.session.active_session.clone();

        // When navigating up.
        let result = navigate(&SidebarIntent::MoveUp, &mut state);

        // Then the result is Moved.
        assert_eq!(result, SectionNavResult::Moved);
        // And the cursor moved to index 1.
        assert_eq!(state.frontend.sessions_section.selected_index, Some(1));
        // And the active session did NOT change.
        assert_eq!(state.session.active_session, original_active);
    }

    #[rstest::rstest]
    fn navigate_down_at_bottom_returns_exhausted() {
        // Given state with 2 sessions, cursor at last index.
        let mut state = state_with_sessions(2);
        let sessions = sorted_open_sessions(&state);
        state.frontend.sessions_section.selected_index = Some(sessions.len() - 1);

        // When navigating down.
        let result = navigate(&SidebarIntent::MoveDown, &mut state);

        // Then the result is Exhausted.
        assert_eq!(result, SectionNavResult::Exhausted);
    }

    #[rstest::rstest]
    fn navigate_up_at_top_returns_exhausted() {
        // Given state with 2 sessions, cursor at index 0.
        let mut state = state_with_sessions(2);
        state.frontend.sessions_section.selected_index = Some(0);

        // When navigating up.
        let result = navigate(&SidebarIntent::MoveUp, &mut state);

        // Then the result is Exhausted.
        assert_eq!(result, SectionNavResult::Exhausted);
    }

    #[rstest::rstest]
    fn navigate_action_returns_moved() {
        let mut state = AppState::default();
        let result = navigate(&SidebarIntent::Action(crate::Intent::Quit), &mut state);
        assert_eq!(result, SectionNavResult::Moved);
    }

    // --- receive_cursor ---

    #[rstest::rstest]
    fn receive_cursor_positions_at_active_session() {
        // Given state with 3 sessions, second one active.
        let mut state = state_with_sessions(3);
        let sessions = sorted_open_sessions(&state);
        // Make the last session active (by ID from original sort).
        let last_id = sessions.iter().find(|s| !s.is_active).unwrap().id.clone();
        state.session.active_session = last_id;

        // When receiving cursor.
        receive_cursor(&mut state, EnterFrom::Top);

        // Then the selected index is 0 (active session sorts to top).
        assert_eq!(state.frontend.sessions_section.selected_index, Some(0));
    }

    // --- sorted_open_sessions ---

    #[rstest::rstest]
    fn sorted_sessions_orders_by_created_at_descending() {
        let state = state_with_sessions(3);
        let sessions = sorted_open_sessions(&state);
        // Sessions are sorted by created_at descending (newest first).
        // The default session (created first) is the oldest, so it's last.
        assert_eq!(sessions.len(), 3);
        assert!(sessions[0].created_at >= sessions[1].created_at);
        assert!(sessions[1].created_at >= sessions[2].created_at);
    }

    #[rstest::rstest]
    fn sorted_sessions_count_matches_hashmap() {
        let state = state_with_sessions(4);
        assert_eq!(sorted_open_sessions(&state).len(), 4);
    }

    // --- Rendering ---

    use nullslop_testutil::setup_term;

    fn render_rows(
        section: &mut SessionsSection,
        state: &AppState,
        width: u16,
        height: u16,
    ) -> Vec<String> {
        let (mut terminal, area) = setup_term(width, height);
        terminal
            .draw(|frame| {
                section.render(frame, area, state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| {
                        buffer
                            .cell((x, y))
                            .map_or(" ", ratatui::buffer::Cell::symbol)
                    })
                    .collect()
            })
            .collect()
    }

    #[rstest::rstest]
    fn render_shows_sessions_header() {
        let mut section = SessionsSection::new();
        let state = AppState::default();
        let rows = render_rows(&mut section, &state, 30, 5);
        assert!(rows[0].contains("Sessions"));
    }

    #[rstest::rstest]
    fn render_shows_active_indicator_on_active_session() {
        let mut section = SessionsSection::new();
        let state = AppState::default();
        let rows = render_rows(&mut section, &state, 30, 5);
        let combined = rows.join("\n");
        assert!(
            combined.contains("▸"),
            "should contain active indicator, got: {combined}"
        );
    }

    #[rstest::rstest]
    fn render_shows_untitled_for_session_without_title() {
        let mut section = SessionsSection::new();
        let state = AppState::default();
        let rows = render_rows(&mut section, &state, 30, 5);
        let combined = rows.join("\n");
        assert!(
            combined.contains("Untitled Session"),
            "should contain 'Untitled Session', got: {combined}"
        );
    }

    // --- Close session ---

    use super::{SessionCloseError, handle_session_close, validate_session_close};

    #[rstest::rstest]
    fn close_session_switches_to_next() {
        // Given state with 3 sessions, sessions section focused, cursor at index 0.
        let mut state = state_with_sessions(3);
        state.frontend.sidebar.focused_section = SidebarSectionId::Sessions;
        let sessions = sorted_open_sessions(&state);
        let closing_id = sessions[0].id.clone();
        state.frontend.sessions_section.selected_index = Some(0);

        // When closing the session.
        handle_session_close(&mut state);

        // Then the closed session is removed.
        assert!(!state.session.sessions.contains_key(&closing_id));
        // And the active session has changed.
        assert_eq!(state.session.sessions.len(), 2);
    }

    #[rstest::rstest]
    fn close_last_session_creates_new() {
        // Given state with 1 session, sessions section focused.
        let mut state = AppState::default();
        state.frontend.sidebar.focused_section = SidebarSectionId::Sessions;
        let original_id = state.session.active_session.clone();
        state.frontend.sessions_section.selected_index = Some(0);

        // When closing the session.
        handle_session_close(&mut state);

        // Then a new session is created.
        assert_eq!(state.session.sessions.len(), 1);
        assert_ne!(state.session.active_session, original_id);
        assert_eq!(state.frontend.sessions_section.selected_index, Some(0));
    }

    #[rstest::rstest]
    fn close_session_clamps_index() {
        // Given state with 3 sessions, sessions section focused, cursor at last index.
        let mut state = state_with_sessions(3);
        state.frontend.sidebar.focused_section = SidebarSectionId::Sessions;
        let sessions = sorted_open_sessions(&state);
        state.session.active_session = sessions[2].id.clone();
        // Move cursor to index 2 (the active session, sorted to 0, so use index 0)
        state.frontend.sessions_section.selected_index = Some(0);

        // When closing.
        handle_session_close(&mut state);

        // Then index is clamped to valid range.
        let selected = state.frontend.sessions_section.selected_index;
        assert!(selected.is_some());
        assert!(selected.unwrap() < state.session.sessions.len());
    }

    #[rstest::rstest]
    fn close_session_rejected_when_streaming() {
        // Given state with a streaming session, sessions section focused.
        let mut state = AppState::default();
        state.frontend.sidebar.focused_section = SidebarSectionId::Sessions;
        state.frontend.sessions_section.selected_index = Some(0);
        state.active_session_mut().begin_streaming();

        // When validating close.
        let result = validate_session_close(&state);

        // Then validation fails with SessionBusy.
        assert_eq!(result, Err(SessionCloseError::SessionBusy));
    }

    #[rstest::rstest]
    fn close_session_rejected_when_wrong_section() {
        // Given state with sessions section NOT focused.
        let state = AppState::default();

        // When validating close.
        let result = validate_session_close(&state);

        // Then validation fails with WrongSection.
        assert_eq!(result, Err(SessionCloseError::WrongSection));
    }

    // --- Activate session ---

    use super::handle_session_activate;

    #[rstest::rstest]
    fn activate_switches_to_cursor_session() {
        // Given state with 3 sessions, sessions section focused, cursor at index 1.
        let mut state = state_with_sessions(3);
        state.frontend.sidebar.focused_section = SidebarSectionId::Sessions;
        let sessions = sorted_open_sessions(&state);
        state.frontend.sessions_section.selected_index = Some(1);
        let target_id = sessions[1].id.clone();

        // When activating.
        handle_session_activate(&mut state);

        // Then the active session is the one at cursor.
        assert_eq!(state.session.active_session, target_id);
    }

    #[rstest::rstest]
    fn activate_is_noop_when_not_sessions_section() {
        // Given state with persona section focused.
        let mut state = state_with_sessions(3);
        let original_active = state.session.active_session.clone();
        state.frontend.sessions_section.selected_index = Some(1);

        // When activating.
        handle_session_activate(&mut state);

        // Then active session is unchanged.
        assert_eq!(state.session.active_session, original_active);
    }
}
