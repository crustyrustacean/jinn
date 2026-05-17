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
/// Maximum number of session entries visible at once.
const MAX_VISIBLE_SESSIONS: usize = 15;

/// Sessions section cursor state — stored on `FrontendState`.
///
/// Tracks the selected index within the sorted open sessions list.
/// `None` means no cursor (section not focused).
#[derive(Debug, Clone, Default)]
pub struct SessionsSectionState {
    /// Index into the sorted open sessions list.
    pub selected_index: Option<usize>,
    /// Scroll offset: the first session entry index that is visible.
    pub scroll_offset: usize,
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

/// Adjusts scroll offset to ensure the selected index is visible within the window.
///
/// If no index is selected, does nothing.
pub fn scroll_to_cursor(state: &mut AppState) {
    let Some(index) = state.frontend.sessions_section.selected_index else {
        return;
    };
    let total = sorted_open_sessions(state).len();
    let visible = MAX_VISIBLE_SESSIONS.min(total);
    if visible == 0 {
        return;
    }
    let offset = &mut state.frontend.sessions_section.scroll_offset;

    if index < *offset {
        *offset = index;
    } else if index >= *offset + visible {
        *offset = index - visible + 1;
    }
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

    let result = match intent {
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
    };

    scroll_to_cursor(state);
    result
}

/// Place the cursor on this section from a given direction.
///
/// Positions at the edge of the list: index 0 from top, last index from bottom.
/// This keeps the linear `j`/`k` scroll model consistent.
pub fn receive_cursor(state: &mut AppState, enter_from: EnterFrom) {
    let sessions = sorted_open_sessions(state);
    if sessions.is_empty() {
        return;
    }
    let index = match enter_from {
        EnterFrom::Top => 0,
        EnterFrom::Bottom => sessions.len() - 1,
    };
    state.frontend.sessions_section.selected_index = Some(index);
    scroll_to_cursor(state);
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
        let scroll_offset = state.frontend.sessions_section.scroll_offset;

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
            let visible_count = MAX_VISIBLE_SESSIONS.min(sessions.len());
            let start = scroll_offset.min(sessions.len());
            let end = (start + visible_count).min(sessions.len());

            for (visual_i, entry) in sessions[start..end].iter().enumerate() {
                let i = start + visual_i; // absolute index for selection check
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

            // Scroll indicators.
            let lines_above = scroll_offset;
            let lines_below = sessions
                .len()
                .saturating_sub(scroll_offset)
                .saturating_sub(visible_count);

            if lines_above > 0 || lines_below > 0 {
                let indicator_style = Style::default().fg(Color::Black).bg(theme.age_fresh);

                if lines_above > 0 {
                    let indicator_row = area.y + 2; // header + blank
                    let label = "\u{2191}"; // ↑
                    let indicator_width = 1u16;
                    let indicator_area = Rect {
                        x: area.x + area.width.saturating_sub(indicator_width),
                        y: indicator_row,
                        width: indicator_width,
                        height: 1,
                    };
                    frame.render_widget(
                        Paragraph::new(Line::from(Span::styled(label, indicator_style))),
                        indicator_area,
                    );
                }

                if lines_below > 0 {
                    let last_entry_row = area.y + 2 + visible_count as u16 - 1;
                    let label = "\u{2193}"; // ↓
                    let indicator_width = 1u16;
                    let indicator_area = Rect {
                        x: area.x + area.width.saturating_sub(indicator_width),
                        y: last_entry_row,
                        width: indicator_width,
                        height: 1,
                    };
                    frame.render_widget(
                        Paragraph::new(Line::from(Span::styled(label, indicator_style))),
                        indicator_area,
                    );
                }
            }
        }

        // Trailing gap.
        lines.push(Line::from(""));

        let widget = Paragraph::new(lines).block(Block::default().borders(Borders::NONE));
        frame.render_widget(widget, area);
    }

    fn content_height(&self, state: &AppState) -> u16 {
        let session_count = state.session.sessions.len() as u16;
        let visible = session_count.min(MAX_VISIBLE_SESSIONS as u16);
        // header(1) + blank(1) + visible sessions(N) + trailing gap(1)
        3 + visible.max(1) // max(1) for "No open sessions" message
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
    let was_active = sessions[index].is_active;

    // Remove from HashMap (keeps in SQLite).
    state.session.sessions.remove(&closing_id);

    if state.session.sessions.is_empty() {
        // Last session — create a new one with the last-used model/strategy.
        let new_session = {
            let model = state
                .frontend
                .preferences
                .last_model
                .clone()
                .unwrap_or_else(|| crate::feat::provider_infra::NO_PROVIDER_ID.to_owned());
            let strategy = state
                .frontend
                .preferences
                .last_strategy
                .as_deref()
                .map_or_else(
                    crate::protocol::PromptStrategyId::passthrough,
                    crate::protocol::PromptStrategyId::new,
                );
            crate::feat::session::chat_session::ChatSessionState::new_with_profile(
                crate::feat::session::profile::SessionProfile::from_config(model, strategy),
            )
        };
        let new_id = new_session.session_id().clone();
        state.session.sessions.insert(new_id.clone(), new_session);
        state.session.active_session = new_id;
        state.frontend.sessions_section.selected_index = Some(0);
    } else if was_active {
        // Closed the active session — activate next one. Clamp index to valid range.
        let remaining = sorted_open_sessions(state);
        let clamped = index.min(remaining.len() - 1);
        state.session.active_session = remaining[clamped].id.clone();
        state.frontend.sessions_section.selected_index = Some(clamped);
    } else {
        // Closed a non-active session — keep active session, clamp cursor.
        let remaining = sorted_open_sessions(state);
        let clamped = index.min(remaining.len() - 1);
        state.frontend.sessions_section.selected_index = Some(clamped);
    }

    scroll_to_cursor(state);

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
    use ratatui::style::Color;

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

    #[rstest::rstest]
    fn content_height_capped_at_max_visible() {
        // Given state with 20 sessions (more than MAX_VISIBLE_SESSIONS = 15).
        let section = SessionsSection::new();
        let state = state_with_sessions(20);

        // When computing content height.
        let height = section.content_height(&state);

        // Then it is capped at 3 + 15 = 18, not 3 + 20 = 23.
        assert_eq!(height, 18);
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

    // --- scroll_to_cursor ---

    use super::scroll_to_cursor;

    #[rstest::rstest]
    fn scroll_to_cursor_adjusts_offset_when_cursor_above_window() {
        // Given 20 sessions with scroll_offset at 5, cursor at index 3.
        let mut state = state_with_sessions(20);
        state.frontend.sessions_section.scroll_offset = 5;
        state.frontend.sessions_section.selected_index = Some(3);

        // When scrolling to cursor.
        scroll_to_cursor(&mut state);

        // Then scroll_offset moves to 3.
        assert_eq!(state.frontend.sessions_section.scroll_offset, 3);
    }

    #[rstest::rstest]
    fn scroll_to_cursor_adjusts_offset_when_cursor_below_window() {
        // Given 20 sessions with scroll_offset at 0, cursor at index 18.
        let mut state = state_with_sessions(20);
        state.frontend.sessions_section.scroll_offset = 0;
        state.frontend.sessions_section.selected_index = Some(18);

        // When scrolling to cursor.
        scroll_to_cursor(&mut state);

        // Then scroll_offset moves to 18 - 15 + 1 = 4.
        assert_eq!(state.frontend.sessions_section.scroll_offset, 4);
    }

    #[rstest::rstest]
    fn scroll_to_cursor_noop_when_cursor_visible() {
        // Given 20 sessions with scroll_offset at 5, cursor at index 10.
        let mut state = state_with_sessions(20);
        state.frontend.sessions_section.scroll_offset = 5;
        state.frontend.sessions_section.selected_index = Some(10);

        // When scrolling to cursor.
        scroll_to_cursor(&mut state);

        // Then scroll_offset stays at 5 (10 is within 5..20).
        assert_eq!(state.frontend.sessions_section.scroll_offset, 5);
    }

    #[rstest::rstest]
    fn scroll_to_cursor_noop_when_no_selection() {
        // Given 20 sessions with no selection.
        let mut state = state_with_sessions(20);
        state.frontend.sessions_section.scroll_offset = 5;
        state.frontend.sessions_section.selected_index = None;

        // When scrolling to cursor.
        scroll_to_cursor(&mut state);

        // Then scroll_offset stays at 5.
        assert_eq!(state.frontend.sessions_section.scroll_offset, 5);
    }

    #[rstest::rstest]
    fn navigate_down_scrolls_viewport_at_bottom() {
        // Given 20 sessions, scroll_offset at 0, cursor at index 14 (last visible).
        let mut state = state_with_sessions(20);
        state.frontend.sessions_section.scroll_offset = 0;
        state.frontend.sessions_section.selected_index = Some(14);

        // When navigating down to index 15.
        navigate(&SidebarIntent::MoveDown, &mut state);

        // Then cursor is at 15 and scroll_offset moved to 1.
        assert_eq!(state.frontend.sessions_section.selected_index, Some(15));
        assert_eq!(state.frontend.sessions_section.scroll_offset, 1);
    }

    #[rstest::rstest]
    fn navigate_up_scrolls_viewport_at_top() {
        // Given 20 sessions, scroll_offset at 5, cursor at index 5.
        let mut state = state_with_sessions(20);
        state.frontend.sessions_section.scroll_offset = 5;
        state.frontend.sessions_section.selected_index = Some(5);

        // When navigating up to index 4.
        navigate(&SidebarIntent::MoveUp, &mut state);

        // Then cursor is at 4 and scroll_offset moved to 4.
        assert_eq!(state.frontend.sessions_section.selected_index, Some(4));
        assert_eq!(state.frontend.sessions_section.scroll_offset, 4);
    }

    // --- receive_cursor ---

    #[rstest::rstest]
    fn receive_cursor_from_top_positions_at_index_zero() {
        // Given state with 3 sessions.
        let mut state = state_with_sessions(3);

        // When receiving cursor from top.
        receive_cursor(&mut state, EnterFrom::Top);

        // Then the selected index is 0.
        assert_eq!(state.frontend.sessions_section.selected_index, Some(0));
    }

    #[rstest::rstest]
    fn receive_cursor_from_bottom_positions_at_last_index() {
        // Given state with 3 sessions.
        let mut state = state_with_sessions(3);
        let count = sorted_open_sessions(&state).len();

        // When receiving cursor from bottom.
        receive_cursor(&mut state, EnterFrom::Bottom);

        // Then the selected index is the last one.
        assert_eq!(
            state.frontend.sessions_section.selected_index,
            Some(count - 1)
        );
    }

    #[rstest::rstest]
    fn receive_cursor_noop_when_empty() {
        // Given state with no sessions (manually clear default).
        let mut state = AppState::default();
        state.session.sessions.clear();

        // When receiving cursor.
        receive_cursor(&mut state, EnterFrom::Top);

        // Then no index is selected.
        assert_eq!(state.frontend.sessions_section.selected_index, None);
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

    #[rstest::rstest]
    fn render_shows_down_arrow_when_entries_hidden_below() {
        // Given 20 sessions with scroll_offset at 0 (15 visible, 5 hidden below).
        let mut section = SessionsSection::new();
        let state = {
            let mut s = state_with_sessions(20);
            s.frontend.sessions_section.scroll_offset = 0;
            s
        };
        // content_height = 3 + 15 = 18, but we'll render in a taller area to be safe.
        let rows = render_rows(&mut section, &state, 30, 20);

        // Then the ↓ indicator appears on the last visible entry row.
        // Row layout: 0=header, 1=blank, 2..16=entries (15), 17=gap.
        // Last entry row is row 16 (index 14 in visible window).
        // Indicator is right-aligned on that row.
        let last_entry_row = &rows[16];
        assert!(
            last_entry_row.contains("\u{2193}"),
            "last entry row should contain ↓, got: {last_entry_row}"
        );
    }

    #[rstest::rstest]
    fn render_shows_up_arrow_when_entries_hidden_above() {
        // Given 20 sessions with scroll_offset at 5 (15 visible, 5 hidden above).
        let mut section = SessionsSection::new();
        let state = {
            let mut s = state_with_sessions(20);
            s.frontend.sessions_section.scroll_offset = 5;
            s
        };
        let rows = render_rows(&mut section, &state, 30, 20);

        // Then the ↑ indicator appears on the first visible entry row (row 2).
        let first_entry_row = &rows[2];
        assert!(
            first_entry_row.contains("\u{2191}"),
            "first entry row should contain ↑, got: {first_entry_row}"
        );
    }

    #[rstest::rstest]
    fn render_shows_both_arrows_when_viewport_in_middle() {
        // Given 20 sessions with scroll_offset at 3 (3 hidden above, 2 hidden below).
        let mut section = SessionsSection::new();
        let state = {
            let mut s = state_with_sessions(20);
            s.frontend.sessions_section.scroll_offset = 3;
            s
        };
        let rows = render_rows(&mut section, &state, 30, 20);

        // Then both indicators appear.
        let first_entry_row = &rows[2];
        let last_entry_row = &rows[16];
        assert!(
            first_entry_row.contains("\u{2191}"),
            "first entry row should contain ↑, got: {first_entry_row}"
        );
        assert!(
            last_entry_row.contains("\u{2193}"),
            "last entry row should contain ↓, got: {last_entry_row}"
        );
    }

    #[rstest::rstest]
    fn render_no_arrows_when_all_entries_visible() {
        // Given 5 sessions (fewer than MAX_VISIBLE_SESSIONS).
        let mut section = SessionsSection::new();
        let state = state_with_sessions(5);
        let rows = render_rows(&mut section, &state, 30, 10);

        // Then no arrow indicators appear on entry rows.
        let combined = rows.join("");
        assert!(
            !combined.contains("\u{2191}") && !combined.contains("\u{2193}"),
            "should not contain scroll indicators, got: {combined}"
        );
    }

    #[rstest::rstest]
    fn render_arrow_has_inverted_colors() {
        // Given 20 sessions with scroll_offset at 0 (↓ indicator visible).
        let mut section = SessionsSection::new();
        let state = {
            let mut s = state_with_sessions(20);
            s.frontend.sessions_section.scroll_offset = 0;
            s
        };
        let (mut terminal, area) = setup_term(30, 20);
        terminal
            .draw(|frame| {
                section.render(frame, area, &state);
            })
            .unwrap();

        // Then the ↓ indicator on row 16 has fg=Black, bg=LightGreen.
        let buffer = terminal.backend().buffer();
        let arrow_cell = buffer.cell((29, 16)).expect("cell should exist");
        assert_eq!(arrow_cell.symbol(), "\u{2193}");
        assert_eq!(arrow_cell.style().fg, Some(Color::Black));
        assert_eq!(arrow_cell.style().bg, Some(Color::LightGreen));
    }

    // --- Close session ---

    use super::{SessionCloseError, handle_session_close, validate_session_close};

    #[rstest::rstest]
    fn close_session_switches_to_next() {
        // Given state with 3 sessions, sessions section focused, cursor at index 0 (active session).
        let mut state = state_with_sessions(3);
        state.frontend.sidebar.focused_section = SidebarSectionId::Sessions;
        let sessions = sorted_open_sessions(&state);
        // Active session is at index 0 (sorted newest-first, default is oldest → last, but we
        // set active to index 0 explicitly to test active-session close).
        state.session.active_session = sessions[0].id.clone();
        let closing_id = sessions[0].id.clone();
        state.frontend.sessions_section.selected_index = Some(0);

        // When closing the active session.
        handle_session_close(&mut state);

        // Then the closed session is removed and active session changed.
        assert!(!state.session.sessions.contains_key(&closing_id));
        assert_eq!(state.session.sessions.len(), 2);
        assert_ne!(state.session.active_session, closing_id);
    }

    #[rstest::rstest]
    fn close_non_active_session_keeps_active() {
        // Given state with 3 sessions, sessions section focused, cursor at index 1 (not active).
        let mut state = state_with_sessions(3);
        state.frontend.sidebar.focused_section = SidebarSectionId::Sessions;
        let sessions = sorted_open_sessions(&state);
        // Active session is at index 0.
        state.session.active_session = sessions[0].id.clone();
        let active_id = state.session.active_session.clone();
        // Close session at index 1 (non-active).
        let closing_id = sessions[1].id.clone();
        state.frontend.sessions_section.selected_index = Some(1);

        // When closing the non-active session.
        handle_session_close(&mut state);

        // Then the closed session is removed.
        assert!(!state.session.sessions.contains_key(&closing_id));
        // And the active session did NOT change.
        assert_eq!(state.session.active_session, active_id);
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
    fn close_session_adjusts_scroll_offset() {
        // Given 20 sessions with scroll_offset at 10, sessions section focused, cursor at 10.
        let mut state = state_with_sessions(20);
        state.frontend.sidebar.focused_section = SidebarSectionId::Sessions;
        state.frontend.sessions_section.scroll_offset = 10;
        state.frontend.sessions_section.selected_index = Some(10);

        // When closing the session at index 10.
        handle_session_close(&mut state);

        // Then scroll_offset is adjusted to keep the cursor visible.
        // After removal there are 19 sessions. The clamped index is 10.
        // scroll_to_cursor ensures index 10 is visible in a window of 15 from offset 10.
        assert_eq!(state.frontend.sessions_section.selected_index, Some(10));
        assert!(state.frontend.sessions_section.scroll_offset <= 10);
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
