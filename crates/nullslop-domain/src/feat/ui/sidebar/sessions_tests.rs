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

    // Then the title text on row 2 (first entry row) has red foreground.
    let buffer = terminal.backend().buffer();
    // The title starts after indicator(1) + space(1) + prefix(2) = column 4.
    let title_cell = buffer.cell((4, 2)).expect("title cell should exist");
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

    // Then the title text on row 2 has the primary_text color (active session).
    let buffer = terminal.backend().buffer();
    let title_cell = buffer.cell((4, 2)).expect("title cell should exist");
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
            setup_command: Some("echo setup".to_owned()),
            teardown_command: Some("cleanup.sh $1".to_owned()),
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
            setup_command: Some("echo setup".to_owned()),
            teardown_command: None,
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
