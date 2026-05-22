#![allow(clippy::expect_used, clippy::indexing_slicing)]

use crate::common::app_state::{AppState, FocusScope};
use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::ui::sidebar::section_trait::{
    EnterFrom, SectionNavResult, SidebarIntent, SidebarSection, SidebarSectionId,
};
use crate::feat::ui::sidebar::sessions::{
    SessionCloseError, SessionsSection, handle_session_activate, handle_session_close, navigate,
    receive_cursor, scroll_to_cursor, sorted_open_sessions, validate_session_close,
};
use crate::protocol::ChatEntry;
use ratatui::style::Color;

// Helper: create state with N sessions.
fn state_with_sessions(count: usize) -> AppState {
    let mut state = AppState::default();
    // Default state already has 1 session. Add more as needed.
    for i in 1..count {
        let session = ChatSessionState::new();
        let id = session.session_id().clone();
        // Give each additional session a title.
        state.session.sessions_mut().insert(id, {
            let mut s = ChatSessionState::new();
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
    assert_eq!(section.content_height(&state), 2); // 1 session + footer
}

#[rstest::rstest]
fn content_height_with_three_sessions() {
    let section = SessionsSection::new();
    let state = state_with_sessions(3);
    assert_eq!(section.content_height(&state), 4); // 3 sessions + footer
}

#[rstest::rstest]
fn content_height_capped_at_max_visible() {
    // Given state with 20 sessions (more than MAX_VISIBLE_SESSIONS = 15).
    let section = SessionsSection::new();
    let state = state_with_sessions(20);

    // When computing content height.
    let height = section.content_height(&state);

    // Then it is capped at 15 + 1 = 16, not 20 + 1 = 21.
    assert_eq!(height, 16);
}

// --- Navigation ---

#[rstest::rstest]
fn navigate_down_moves_cursor_without_switching() {
    // Given state with 3 sessions, cursor at index 0.
    let mut state = state_with_sessions(3);
    let original_active = state.session.active_session_id().clone();
    state.frontend.sessions_section.selected_index = Some(0);

    // When navigating down.
    let result = navigate(&SidebarIntent::MoveDown, &mut state);

    // Then the result is Moved.
    assert_eq!(result, SectionNavResult::Moved);
    // And the cursor moved to index 1.
    assert_eq!(state.frontend.sessions_section.selected_index, Some(1));
    // And the active session did NOT change.
    assert_eq!(*state.session.active_session_id(), original_active);
}

#[rstest::rstest]
fn navigate_up_moves_cursor_without_switching() {
    // Given state with 3 sessions, cursor at index 2.
    let mut state = state_with_sessions(3);
    let sessions = sorted_open_sessions(&state);
    state.session.set_active(sessions[2].id.clone());
    state.frontend.sessions_section.selected_index = Some(2);
    let original_active = state.session.active_session_id().clone();

    // When navigating up.
    let result = navigate(&SidebarIntent::MoveUp, &mut state);

    // Then the result is Moved.
    assert_eq!(result, SectionNavResult::Moved);
    // And the cursor moved to index 1.
    assert_eq!(state.frontend.sessions_section.selected_index, Some(1));
    // And the active session did NOT change.
    assert_eq!(*state.session.active_session_id(), original_active);
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
    state.session.sessions_mut().clear();

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
fn render_shows_sessions_footer() {
    // Given a sessions section with default state.
    let mut section = SessionsSection::new();
    let state = AppState::default();

    // When rendering.
    let rows = render_rows(&mut section, &state, 30, 5);

    // Then the last row contains "Sessions" (footer at the bottom).
    let combined = rows.join("\n");
    assert!(
        combined.contains("Sessions"),
        "should contain 'Sessions' in footer, got: {combined}"
    );
}

#[rstest::rstest]
fn render_shows_active_indicator_on_active_session() {
    let mut section = SessionsSection::new();
    let state = AppState::default();
    let rows = render_rows(&mut section, &state, 30, 5);
    let combined = rows.join("\n");
    assert!(
        combined.contains("\u{25b8}"),
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
    // Row layout: 0..14=entries (15), 15=footer.
    // Last entry row is row 14 (index 14 in visible window).
    let last_entry_row = &rows[14];
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

    // Then the ↑ indicator appears on the first visible entry row (row 0).
    let first_entry_row = &rows[0];
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
    let first_entry_row = &rows[0];
    let last_entry_row = &rows[14];
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

    // Then the ↓ indicator on row 14 has fg=Black, bg=LightGreen.
    // Row layout: 0..14=entries (15), 15=footer.
    let buffer = terminal.backend().buffer();
    let arrow_cell = buffer.cell((29, 14)).expect("cell should exist");
    assert_eq!(arrow_cell.symbol(), "\u{2193}");
    assert_eq!(arrow_cell.style().fg, Some(Color::Black));
    assert_eq!(arrow_cell.style().bg, Some(Color::LightGreen));
}

// --- Close session ---

#[rstest::rstest]
fn close_session_switches_to_next() {
    // Given state with 3 sessions, sessions section focused, cursor at index 0 (active session).
    let mut state = state_with_sessions(3);
    state.frontend.scope_stack.push(FocusScope::SidebarSessions);
    let sessions = sorted_open_sessions(&state);
    // Active session is at index 0 (sorted newest-first, default is oldest → last, but we
    // set active to index 0 explicitly to test active-session close).
    state.session.set_active(sessions[0].id.clone());
    let closing_id = sessions[0].id.clone();
    state.frontend.sessions_section.selected_index = Some(0);

    // When closing the active session.
    handle_session_close(&mut state);

    // Then the closed session is removed and active session changed.
    assert!(!state.session.sessions().contains_key(&closing_id));
    assert_eq!(state.session.sessions().len(), 2);
    assert_ne!(*state.session.active_session_id(), closing_id);
}

#[rstest::rstest]
fn close_non_active_session_keeps_active() {
    // Given state with 3 sessions, sessions section focused, cursor at index 1 (not active).
    let mut state = state_with_sessions(3);
    state.frontend.scope_stack.push(FocusScope::SidebarSessions);
    let sessions = sorted_open_sessions(&state);
    // Active session is at index 0.
    state.session.set_active(sessions[0].id.clone());
    let active_id = state.session.active_session_id().clone();
    // Close session at index 1 (non-active).
    let closing_id = sessions[1].id.clone();
    state.frontend.sessions_section.selected_index = Some(1);

    // When closing the non-active session.
    handle_session_close(&mut state);

    // Then the closed session is removed.
    assert!(!state.session.sessions().contains_key(&closing_id));
    // And the active session did NOT change.
    assert_eq!(*state.session.active_session_id(), active_id);
}

#[rstest::rstest]
fn close_last_session_creates_new() {
    // Given state with 1 session, sessions section focused.
    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::SidebarSessions);
    let original_id = state.session.active_session_id().clone();
    state.frontend.sessions_section.selected_index = Some(0);

    // When closing the session.
    handle_session_close(&mut state);

    // Then a new session is created.
    assert_eq!(state.session.sessions().len(), 1);
    assert_ne!(*state.session.active_session_id(), original_id);
    assert_eq!(state.frontend.sessions_section.selected_index, Some(0));
}

#[rstest::rstest]
fn close_session_clamps_index() {
    // Given state with 3 sessions, sessions section focused, cursor at last index.
    let mut state = state_with_sessions(3);
    state.frontend.scope_stack.push(FocusScope::SidebarSessions);
    let sessions = sorted_open_sessions(&state);
    state.session.set_active(sessions[2].id.clone());
    // Move cursor to index 2 (the active session, sorted to 0, so use index 0)
    state.frontend.sessions_section.selected_index = Some(0);

    // When closing.
    handle_session_close(&mut state);

    // Then index is clamped to valid range.
    let selected = state.frontend.sessions_section.selected_index;
    assert!(selected.is_some());
    assert!(selected.unwrap() < state.session.sessions().len());
}

#[rstest::rstest]
fn close_session_adjusts_scroll_offset() {
    // Given 20 sessions with scroll_offset at 10, sessions section focused, cursor at 10.
    let mut state = state_with_sessions(20);
    state.frontend.scope_stack.push(FocusScope::SidebarSessions);
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
    state.frontend.scope_stack.push(FocusScope::SidebarSessions);
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

// --- Error-colored session titles ---

#[rstest::rstest]
fn render_session_title_is_red_when_last_entry_is_error() {
    // Given a session whose last history entry is an error.
    let mut section = SessionsSection::new();
    let state = {
        let mut s = AppState::default();
        // Push an error entry into the active (only) session.
        s.active_session_mut()
            .push_entry(ChatEntry::error("teardown failed"));
        s
    };

    // When rendering.
    let (mut terminal, area) = setup_term(30, 5);
    terminal
        .draw(|frame| {
            section.render(frame, area, &state);
        })
        .unwrap();

    // Then the title text on row 0 (first entry row) has red foreground.
    let buffer = terminal.backend().buffer();
    // The title starts after indicator(1) + space(1) + prefix(2) = column 4.
    let title_cell = buffer.cell((4, 0)).expect("title cell should exist");
    assert_eq!(title_cell.style().fg, Some(Color::Red));
}

#[rstest::rstest]
fn render_session_title_is_normal_when_last_entry_is_not_error() {
    // Given a session whose last history entry is a user message (not error).
    let mut section = SessionsSection::new();
    let state = {
        let mut s = AppState::default();
        s.active_session_mut().push_entry(ChatEntry::user("hello"));
        s
    };
    let primary_text = state.frontend.theme.primary_text;

    // When rendering.
    let (mut terminal, area) = setup_term(30, 5);
    terminal
        .draw(|frame| {
            section.render(frame, area, &state);
        })
        .unwrap();

    // Then the title text on row 0 has the primary_text color (active session).
    let buffer = terminal.backend().buffer();
    let title_cell = buffer.cell((4, 0)).expect("title cell should exist");
    assert_eq!(title_cell.style().fg, Some(primary_text));
}

#[rstest::rstest]
fn sorted_sessions_reports_last_entry_is_error() {
    // Given a session whose last entry is an error.
    let mut state = AppState::default();
    state
        .active_session_mut()
        .push_entry(ChatEntry::error("boom"));

    // When collecting sorted sessions.
    let sessions = sorted_open_sessions(&state);

    // Then the entry has last_entry_is_error = true.
    assert!(sessions[0].last_entry_is_error);
}

#[rstest::rstest]
fn sorted_sessions_reports_last_entry_not_error() {
    // Given a session whose last entry is a user message.
    let mut state = AppState::default();
    state
        .active_session_mut()
        .push_entry(ChatEntry::user("hello"));

    // When collecting sorted sessions.
    let sessions = sorted_open_sessions(&state);

    // Then the entry has last_entry_is_error = false.
    assert!(!sessions[0].last_entry_is_error);
}

#[rstest::rstest]
fn sorted_sessions_empty_history_is_not_error() {
    // Given a session with no history entries.
    let state = AppState::default();

    // When collecting sorted sessions.
    let sessions = sorted_open_sessions(&state);

    // Then the entry has last_entry_is_error = false.
    assert!(!sessions[0].last_entry_is_error);
}

// --- Activate session ---

#[rstest::rstest]
fn activate_switches_to_cursor_session() {
    // Given state with 3 sessions, sessions section focused, cursor at index 1.
    let mut state = state_with_sessions(3);
    state.frontend.scope_stack.push(FocusScope::SidebarSessions);
    let sessions = sorted_open_sessions(&state);
    state.frontend.sessions_section.selected_index = Some(1);
    let target_id = sessions[1].id.clone();

    // When activating.
    handle_session_activate(&mut state);

    // Then the active session is the one at cursor.
    assert_eq!(*state.session.active_session_id(), target_id);
}

#[rstest::rstest]
fn activate_is_noop_when_not_sessions_section() {
    // Given state with persona section focused.
    let mut state = state_with_sessions(3);
    let original_active = state.session.active_session_id().clone();
    state.frontend.sessions_section.selected_index = Some(1);

    // When activating.
    handle_session_activate(&mut state);

    // Then active session is unchanged.
    assert_eq!(*state.session.active_session_id(), original_active);
}

// --- SidebarSessionNewWithLifecycle ---

#[rstest::rstest]
fn new_with_lifecycle_noop_when_not_sessions_section() {
    // Given sidebar focused on persona section.
    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::SidebarPersona);

    // When handling the intent via IntentHandler.
    let result = crate::feat::intent::IntentHandler::handle(
        &crate::Intent::SidebarSessionNewWithLifecycle,
        &mut state,
    );

    // Then no picker is opened (scope stays sidebar, no picker overlay).
    assert!(!state.frontend.scope_stack.is_picker());
    // And no commands emitted.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn new_with_lifecycle_opens_picker_when_sessions_section() {
    // Given sidebar focused on sessions section.
    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::SidebarSessions);
    state
        .frontend
        .scope_stack
        .push(crate::common::app_state::FocusScope::SidebarSessions);

    // When handling the intent via IntentHandler.
    let result = crate::feat::intent::IntentHandler::handle(
        &crate::Intent::SidebarSessionNewWithLifecycle,
        &mut state,
    );

    // Then the picker scope is pushed with SessionLifecycle kind.
    assert!(state.frontend.scope_stack.is_picker());
    assert_eq!(
        state.frontend.scope_stack.picker_kind(),
        Some(&crate::protocol::PickerKind::SessionLifecycle)
    );
    // And no commands emitted (lifecycle entries are loaded synchronously).
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn teardown_only_emits_run_session_teardown() {
    // Given a session with a lifecycle that has a teardown command.
    let mut state = AppState::default();
    state.frontend.preferences.session_lifecycles.push(
        crate::feat::preferences_actor::user_preferences::SessionLifecycle {
            name: "fossil branch".to_owned(),
            description: None,
            setup: Some(crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell("echo setup".to_owned())),
            teardown: Some(crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell("cleanup.sh $1".to_owned())),
        },
    );
    state
        .active_session_mut()
        .set_lifecycle_name(Some("fossil branch".to_owned()));
    state
        .active_session_mut()
        .set_lifecycle_args(vec!["my-branch".to_owned()]);
    state.frontend.sessions_section.selected_index = Some(0);
    state
        .frontend
        .scope_stack
        .push(crate::common::app_state::FocusScope::SidebarSessions);

    // When handling SidebarSessionTeardown via IntentHandler.
    let result = crate::feat::intent::IntentHandler::handle(
        &crate::Intent::SidebarSessionTeardown,
        &mut state,
    );

    // Then a RunSessionTeardown command is emitted with the rendered teardown command.
    assert_eq!(result.commands.len(), 1);
    assert!(matches!(
        &result.commands[0],
        crate::protocol::Command::RunSessionTeardown(
            crate::feat::session_lifecycle::protocol::command::RunSessionTeardown {
                command,
                args,
                ..
            }
        ) if command == "cleanup.sh my-branch" && args == &["my-branch".to_owned()]
    ));
}

#[rstest::rstest]
fn teardown_only_is_noop_without_lifecycle_teardown() {
    // Given a session with a lifecycle that has NO teardown command.
    let mut state = AppState::default();
    state.frontend.preferences.session_lifecycles.push(
        crate::feat::preferences_actor::user_preferences::SessionLifecycle {
            name: "plain".to_owned(),
            description: None,
            setup: Some(crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell("echo setup".to_owned())),
            teardown: None,
        },
    );
    state
        .active_session_mut()
        .set_lifecycle_name(Some("plain".to_owned()));
    state.frontend.sessions_section.selected_index = Some(0);
    state
        .frontend
        .scope_stack
        .push(crate::common::app_state::FocusScope::SidebarSessions);

    // When handling SidebarSessionTeardown via IntentHandler.
    let result = crate::feat::intent::IntentHandler::handle(
        &crate::Intent::SidebarSessionTeardown,
        &mut state,
    );

    // Then no commands are emitted (no teardown command to run).
    assert!(result.commands.is_empty());
}

// ---------------------------------------------------------------------------
// Pure render helper tests
// ---------------------------------------------------------------------------

use crate::feat::ui::sidebar::sessions::render::entry_line::{
    arrow_span, entry_title_style, indicator_span,
};
use crate::feat::ui::sidebar::sessions::render::truncate::truncate_str;
use ratatui::style::Modifier;
use throbber_widgets_tui::ThrobberState;

fn default_theme() -> crate::feat::theme::Theme {
    AppState::default().frontend.theme
}

// --- entry_title_style ---

#[rstest::rstest]
fn title_style_is_red_reversed_when_error_and_selected() {
    // Given an entry with error and selected.
    let theme = default_theme();

    // When computing title style.
    let style = entry_title_style(true, false, true, &theme);

    // Then the style is red + reversed.
    assert_eq!(style.fg, Some(Color::Red));
    assert!(style.add_modifier.contains(Modifier::REVERSED));
}

#[rstest::rstest]
fn title_style_is_red_when_error_not_selected() {
    // Given an entry with error but not selected.
    let theme = default_theme();

    // When computing title style.
    let style = entry_title_style(false, false, true, &theme);

    // Then the style is red, no reversed.
    assert_eq!(style.fg, Some(Color::Red));
    assert!(!style.add_modifier.contains(Modifier::REVERSED));
}

#[rstest::rstest]
fn title_style_is_reversed_when_selected_no_error() {
    // Given a selected entry without error.
    let theme = default_theme();

    // When computing title style.
    let style = entry_title_style(true, false, false, &theme);

    // Then the style is reversed, no specific fg.
    assert!(style.add_modifier.contains(Modifier::REVERSED));
    assert_eq!(style.fg, None);
}

#[rstest::rstest]
fn title_style_is_primary_text_when_active_not_selected() {
    // Given an active, not selected entry without error.
    let theme = default_theme();

    // When computing title style.
    let style = entry_title_style(false, true, false, &theme);

    // Then the style has primary text fg.
    assert_eq!(style.fg, Some(theme.primary_text));
}

#[rstest::rstest]
fn title_style_is_muted_text_when_inactive_not_selected() {
    // Given an inactive, not selected entry without error.
    let theme = default_theme();

    // When computing title style.
    let style = entry_title_style(false, false, false, &theme);

    // Then the style has muted text fg.
    assert_eq!(style.fg, Some(theme.muted_text));
}

// --- indicator_span ---

#[rstest::rstest]
fn indicator_span_returns_blank_space_when_idle() {
    // Given an idle entry.
    let throbber = ThrobberState::default();

    // When computing indicator span.
    let span = indicator_span(true, &throbber);

    // Then it is a blank space.
    assert_eq!(span.content, " ");
}

#[rstest::rstest]
fn indicator_span_returns_throbber_character_when_working() {
    // Given a working entry (not idle).
    let throbber = ThrobberState::default();

    // When computing indicator span.
    let span = indicator_span(false, &throbber);

    // Then it is a non-space character with Cyan fg.
    assert_ne!(span.content, " ");
    assert!(!span.content.is_empty());
    assert_eq!(span.style.fg, Some(Color::Cyan));
}

// --- arrow_span ---

#[rstest::rstest]
fn arrow_span_returns_active_prefix_when_active() {
    // Given an active session.
    let theme = default_theme();

    // When computing arrow span.
    let span = arrow_span(true, &theme);

    // Then it contains the active prefix.
    assert_eq!(span.content, "\u{25b8} ");
}

#[rstest::rstest]
fn arrow_span_returns_inactive_prefix_when_not_active() {
    // Given an inactive session.
    let theme = default_theme();

    // When computing arrow span.
    let span = arrow_span(false, &theme);

    // Then it contains the inactive prefix (two spaces).
    assert_eq!(span.content, "  ");
}

// --- truncate_str ---

#[rstest::rstest]
fn truncate_str_returns_original_when_short() {
    // Given a string that fits within max_len.
    let s = "hello";

    // When truncating with max_len = 10.
    let result = truncate_str(s, 10);

    // Then the original string is returned.
    assert_eq!(result, "hello");
}

#[rstest::rstest]
fn truncate_str_appends_ellipsis_when_long() {
    // Given a string that exceeds max_len.
    let s = "hello world";

    // When truncating with max_len = 5.
    let result = truncate_str(s, 5);

    // Then the result is 5 graphemes ending with ellipsis.
    assert_eq!(result, "hell\u{2026}");
}

#[rstest::rstest]
fn truncate_str_returns_empty_when_max_len_zero() {
    // Given max_len of zero.
    let s = "hello";

    // When truncating with max_len = 0.
    let result = truncate_str(s, 0);

    // Then an empty string is returned.
    assert_eq!(result, "");
}

// ---------------------------------------------------------------------------
// Tree integration tests
// ---------------------------------------------------------------------------

/// Helper: create a state with a known parent-child tree.
///
/// Creates:
/// - root_a (oldest root)
///   - child_a1 (oldest child of root_a)
///     - grandchild_a1a
///   - child_a2
/// - root_b (newest root)
fn state_with_tree() -> AppState {
    let mut state = AppState::default();

    // Create root_a with a title.
    let mut root_a = ChatSessionState::new();
    root_a.push_entry(ChatEntry::user("root a"));
    root_a.set_title("root a".to_owned());
    let root_a_id = root_a.session_id().clone();
    state
        .session
        .sessions_mut()
        .insert(root_a_id.clone(), root_a);

    // Create child_a1 under root_a.
    let mut child_a1 = ChatSessionState::new();
    child_a1.set_title("child a1".to_owned());
    child_a1.set_parent_session(root_a_id.clone());
    let child_a1_id = child_a1.session_id().clone();
    state
        .session
        .sessions_mut()
        .insert(child_a1_id.clone(), child_a1);

    // Create grandchild_a1a under child_a1.
    let mut grandchild = ChatSessionState::new();
    grandchild.set_title("grandchild a1a".to_owned());
    grandchild.set_parent_session(child_a1_id.clone());
    let grandchild_id = grandchild.session_id().clone();
    state
        .session
        .sessions_mut()
        .insert(grandchild_id, grandchild);

    // Create child_a2 under root_a.
    let mut child_a2 = ChatSessionState::new();
    child_a2.set_title("child a2".to_owned());
    child_a2.set_parent_session(root_a_id.clone());
    let child_a2_id = child_a2.session_id().clone();
    state.session.sessions_mut().insert(child_a2_id, child_a2);

    // Create root_b with a title (newest root).
    let mut newest_root = ChatSessionState::new();
    newest_root.push_entry(ChatEntry::user("root b"));
    newest_root.set_title("root b".to_owned());
    let newest_root_id = newest_root.session_id().clone();
    state
        .session
        .sessions_mut()
        .insert(newest_root_id.clone(), newest_root);

    // Remove the default session (created at AppState::default).
    let default_id = state.session.active_session_id().clone();
    if default_id != root_a_id && default_id != newest_root_id {
        state.session.sessions_mut().remove(&default_id);
    }

    // Set active to root_b.
    state.session.set_active(newest_root_id);

    state
}

// --- Tree ordering ---

#[rstest::rstest]
fn tree_roots_sorted_by_created_at_descending() {
    // Given state with two root sessions.
    let state = state_with_tree();

    // When collecting sorted sessions.
    let sessions = sorted_open_sessions(&state);

    // Then the roots appear first and are ordered newest-first.
    // root_b was created last, root_a was created first.
    let roots: Vec<_> = sessions.iter().filter(|s| s.depth == 0).collect();
    assert_eq!(roots.len(), 2, "should have 2 roots");
    assert!(
        roots[0].created_at >= roots[1].created_at,
        "roots should be sorted newest-first"
    );
}

#[rstest::rstest]
fn tree_children_sorted_by_created_at_ascending_under_parent() {
    // Given state with root_a having two children.
    let state = state_with_tree();

    // When collecting sorted sessions.
    let sessions = sorted_open_sessions(&state);

    // Find root_a's children (depth 1, parent is root_a).
    let root_a_id = sessions
        .iter()
        .find(|s| s.title.contains("root a"))
        .map(|s| s.id.clone())
        .expect("root a should exist");
    let children: Vec<_> = sessions
        .iter()
        .filter(|s| s.parent_id.as_ref() == Some(&root_a_id))
        .collect();

    // Then children are sorted oldest-first.
    assert_eq!(children.len(), 2, "root_a should have 2 children");
    assert!(
        children[0].created_at <= children[1].created_at,
        "children should be sorted oldest-first"
    );
}

#[rstest::rstest]
fn tree_dfs_order_is_correct() {
    // Given state with root_a -> child_a1 -> grandchild_a1a, child_a2, root_b.
    let state = state_with_tree();

    // When collecting sorted sessions.
    let sessions = sorted_open_sessions(&state);

    // Then DFS order is: root_b (newest root), root_a, child_a1, grandchild_a1a, child_a2.
    // Or root_a first, depending on creation timing.
    // The invariant is: root_a appears before its children, child_a1 appears before grandchild_a1a.
    let root_a_pos = sessions
        .iter()
        .position(|s| s.title.contains("root a"))
        .expect("root a");
    let child_a1_pos = sessions
        .iter()
        .position(|s| s.title.contains("child a1"))
        .expect("child a1");
    let grandchild_pos = sessions
        .iter()
        .position(|s| s.title.contains("grandchild"))
        .expect("grandchild");
    let child_a2_pos = sessions
        .iter()
        .position(|s| s.title.contains("child a2"))
        .expect("child a2");

    assert!(root_a_pos < child_a1_pos, "root_a before child_a1");
    assert!(child_a1_pos < grandchild_pos, "child_a1 before grandchild");
    assert!(
        grandchild_pos < child_a2_pos,
        "grandchild before child_a2 (DFS)"
    );
}

#[rstest::rstest]
fn orphan_session_appears_as_root() {
    // Given a session with a parent that is not loaded.
    let mut state = AppState::default();
    let mut orphan = ChatSessionState::new();
    orphan.set_title("orphan".to_owned());
    orphan.set_parent_session(crate::protocol::SessionId::new());
    state
        .session
        .sessions_mut()
        .insert(orphan.session_id().clone(), orphan);

    // When collecting sorted sessions.
    let sessions = sorted_open_sessions(&state);

    // Then the orphan appears as a root (depth 0).
    let orphan_entry = sessions.iter().find(|s| s.title.contains("orphan"));
    assert!(orphan_entry.is_some(), "orphan should appear");
    assert_eq!(
        orphan_entry.unwrap().depth,
        0,
        "orphan should be treated as root"
    );
}

// --- Navigation through tree ---

#[rstest::rstest]
fn navigate_down_from_root_goes_to_first_child() {
    // Given state with a tree and cursor on root_a.
    let mut state = state_with_tree();
    let sessions = sorted_open_sessions(&state);
    let root_a_index = sessions
        .iter()
        .position(|s| s.title.contains("root a"))
        .expect("root a");
    state.frontend.sessions_section.selected_index = Some(root_a_index);

    // When navigating down.
    navigate(&SidebarIntent::MoveDown, &mut state);

    // Then the cursor is on the next entry (root_a's first child in DFS order).
    let new_sessions = sorted_open_sessions(&state);
    let new_index = state.frontend.sessions_section.selected_index.unwrap();
    assert_eq!(
        new_index,
        root_a_index + 1,
        "cursor should move to next DFS entry"
    );
    // And the entry is child_a1.
    assert!(
        new_sessions[new_index].title.contains("child a1"),
        "next entry should be child_a1"
    );
}

#[rstest::rstest]
fn navigate_up_from_child_goes_to_parent() {
    // Given state with a tree and cursor on child_a1.
    let mut state = state_with_tree();
    let sessions = sorted_open_sessions(&state);
    let child_a1_index = sessions
        .iter()
        .position(|s| s.title.contains("child a1"))
        .expect("child a1");
    state.frontend.sessions_section.selected_index = Some(child_a1_index);

    // When navigating up.
    navigate(&SidebarIntent::MoveUp, &mut state);

    // Then the cursor is on root_a (parent).
    let new_index = state.frontend.sessions_section.selected_index.unwrap();
    assert_eq!(
        new_index,
        child_a1_index - 1,
        "cursor should move to parent"
    );
    let new_sessions = sorted_open_sessions(&state);
    assert!(
        new_sessions[new_index].title.contains("root a"),
        "previous entry should be root_a"
    );
}

// --- Close sessions in tree ---

#[rstest::rstest]
fn close_child_session_clamps_cursor() {
    // Given state with a tree, cursor on child_a1.
    let mut state = state_with_tree();
    state.frontend.scope_stack.push(FocusScope::SidebarSessions);
    let sessions = sorted_open_sessions(&state);
    let child_a1_index = sessions
        .iter()
        .position(|s| s.title.contains("child a1"))
        .expect("child a1");
    let child_a1_id = sessions[child_a1_index].id.clone();
    state.frontend.sessions_section.selected_index = Some(child_a1_index);

    // When closing child_a1.
    handle_session_close(&mut state);

    // Then child_a1 is removed.
    assert!(!state.session.sessions().contains_key(&child_a1_id));
    // And the cursor is clamped to valid range.
    let remaining = sorted_open_sessions(&state);
    let selected = state.frontend.sessions_section.selected_index.unwrap();
    assert!(selected < remaining.len());
}

#[rstest::rstest]
fn close_root_session_promotes_children_to_roots() {
    // Given state with root_a having children.
    let mut state = state_with_tree();
    state.frontend.scope_stack.push(FocusScope::SidebarSessions);
    let sessions = sorted_open_sessions(&state);
    let root_a_index = sessions
        .iter()
        .position(|s| s.title.contains("root a"))
        .expect("root a");
    let root_a_id = sessions[root_a_index].id.clone();
    state.frontend.sessions_section.selected_index = Some(root_a_index);

    // When closing root_a.
    handle_session_close(&mut state);

    // Then root_a is removed.
    assert!(!state.session.sessions().contains_key(&root_a_id));
    // And its former children are now orphans (roots in the new tree).
    let remaining = sorted_open_sessions(&state);
    let former_children: Vec<_> = remaining
        .iter()
        .filter(|s| s.title.contains("child a") || s.title.contains("grandchild"))
        .collect();
    assert!(
        !former_children.is_empty(),
        "former children should still be present"
    );
    // All former children should now be roots or have adjusted depth.
    // The children of root_a become orphans → treated as roots.
    let child_a1_entry = remaining.iter().find(|s| s.title.contains("child a1"));
    assert!(child_a1_entry.is_some(), "child_a1 should still exist");
    assert_eq!(
        child_a1_entry.unwrap().depth,
        0,
        "child_a1 should now be a root (orphan)"
    );
}

// --- Activate child session ---

#[rstest::rstest]
fn activate_child_session_switches_active() {
    // Given state with a tree, cursor on child_a1.
    let mut state = state_with_tree();
    state.frontend.scope_stack.push(FocusScope::SidebarSessions);
    let sessions = sorted_open_sessions(&state);
    let child_a1_index = sessions
        .iter()
        .position(|s| s.title.contains("child a1"))
        .expect("child a1");
    let child_a1_id = sessions[child_a1_index].id.clone();
    state.frontend.sessions_section.selected_index = Some(child_a1_index);

    // When activating.
    handle_session_activate(&mut state);

    // Then the active session is child_a1.
    assert_eq!(*state.session.active_session_id(), child_a1_id);
}

// --- Render tree with mixed depths ---

#[rstest::rstest]
fn render_tree_shows_tree_characters() {
    // Given state with a tree.
    let mut section = SessionsSection::new();
    let state = state_with_tree();

    // When rendering.
    let (mut terminal, area) = nullslop_testutil::setup_term(30, 15);
    terminal
        .draw(|frame| {
            section.render(frame, area, &state);
        })
        .unwrap();

    // Then the buffer contains tree connector characters.
    let buffer = terminal.backend().buffer();
    let text: String = (0..15)
        .flat_map(|y| {
            (0..30).map(move |x| {
                buffer
                    .cell((x, y))
                    .map_or(' ', |c| c.symbol().chars().next().unwrap_or(' '))
            })
        })
        .collect();
    assert!(
        text.contains('├') || text.contains('└'),
        "rendered output should contain tree connectors, got: {text}"
    );
}
