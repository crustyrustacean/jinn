#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

use crate::common::app_state::{AppState, FocusScope};
use crate::common::render_ctx::RenderCtx;
use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::ui::sidebar::section_trait::{
    EnterFrom, SectionNavResult, SidebarIntent, SidebarSection, SidebarSectionId,
};
use crate::feat::ui::sidebar::sessions::state::SessionEntryKind;
use crate::feat::ui::sidebar::sessions::{
    SessionCloseError, SessionsSection, handle_session_activate, handle_session_close, navigate,
    receive_cursor, scroll_to_cursor, sorted_open_sessions, validate_session_close,
};

use crate::protocol::ChatEntry;
use ratatui::style::Color;

/// Helper: get a session title from the live session map via a tree entry.
fn entry_title(state: &AppState, id: &crate::protocol::SessionId) -> String {
    state
        .session
        .get(id)
        .map(|s| s.title().unwrap_or("Untitled Session").to_owned())
        .unwrap_or_default()
}

/// Helper: get a session's created_at from the live session map.
fn entry_created_at(state: &AppState, id: &crate::protocol::SessionId) -> jiff::Timestamp {
    state
        .session
        .get(id)
        .map(|s| *s.created_at())
        .unwrap_or_default()
}

/// Helper: check if the session's last entry is an error.
fn entry_last_is_error(state: &AppState, id: &crate::protocol::SessionId) -> bool {
    state.session.get(id).is_some_and(|s| {
        s.history()
            .last()
            .is_some_and(|e| matches!(&e.kind, crate::protocol::ChatEntryKind::Error(..)))
    })
}

// Helper: create state with N sessions.
fn state_with_sessions(count: usize) -> AppState {
    let mut state = AppState::default();
    // Default state already has 1 session. Add more as needed.
    for i in 1..count {
        let session = ChatSessionState::new();
        let _id = session.session_id().clone();
        // Give each additional session a title.
        state.session.insert({
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
    assert_eq!(section.content_height(&{ RenderCtx::new(&state) }), 2); // 1 session + footer
}

#[rstest::rstest]
fn content_height_with_three_sessions() {
    let section = SessionsSection::new();
    let state = state_with_sessions(3);
    assert_eq!(section.content_height(&{ RenderCtx::new(&state) }), 4); // 3 sessions + footer
}

#[rstest::rstest]
fn content_height_capped_at_max_visible() {
    // Given state with 20 sessions (more than MAX_VISIBLE_SESSIONS = 15).
    let section = SessionsSection::new();
    let state = state_with_sessions(20);

    // When computing content height.
    let height = section.content_height(&{ RenderCtx::new(&state) });

    // Then it is capped at 15 + 1 = 16, not 20 + 1 = 21.
    assert_eq!(height, 16);
}
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
fn scroll_to_cursor_clamps_offset_when_list_shrinks() {
    // Given 20 sessions with scroll_offset at 5, cursor at index 10.
    let mut state = state_with_sessions(20);
    state.frontend.sessions_section.scroll_offset = 5;
    state.frontend.sessions_section.selected_index = Some(10);

    // Remove 15 sessions, leaving only 5.
    let sorted = sorted_open_sessions(&state);
    for entry in &sorted[5..] {
        state.session.remove_without_replacement(&entry.id);
    }
    // Cursor is now clamped to index 4 by the caller (reconcile).
    state.frontend.sessions_section.selected_index = Some(4);

    // When scrolling to cursor.
    scroll_to_cursor(&mut state);

    // Then scroll_offset is clamped to 0 (5 sessions fit in MAX_VISIBLE_SESSIONS).
    assert_eq!(state.frontend.sessions_section.scroll_offset, 0);
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
    // Then sessions are sorted by created_at descending (newest first).
    // Read created_at from live sessions, not from the tree entry.
    assert_eq!(sessions.len(), 3);
    let a = state
        .session
        .get(&sessions[0].id)
        .map(|s| *s.created_at())
        .unwrap_or_default();
    let b = state
        .session
        .get(&sessions[1].id)
        .map(|s| *s.created_at())
        .unwrap_or_default();
    let c = state
        .session
        .get(&sessions[2].id)
        .map(|s| *s.created_at())
        .unwrap_or_default();
    assert!(a >= b);
    assert!(b >= c);
}

#[rstest::rstest]
fn sorted_sessions_count_matches_hashmap() {
    let state = state_with_sessions(4);
    assert_eq!(sorted_open_sessions(&state).len(), 4);
}

#[rstest::rstest]
fn busy_session_is_not_idle() {
    // Given a session that has active busy operations.
    let mut state = AppState::default();
    state.active_session_mut().begin_busy();

    // When collecting sorted open sessions.
    let sessions = sorted_open_sessions(&state);

    // Then the session entry is not idle (busy_count > 0 shows throbber).
    assert_eq!(sessions.len(), 1);
    assert!(
        !sessions[0].is_idle,
        "busy session should show throbber in sidebar"
    );
}

#[rstest::rstest]
fn idle_and_not_busy_is_idle() {
    // Given a session with no busy operations and phase Idle.
    let state = AppState::default();

    // When collecting sorted open sessions.
    let sessions = sorted_open_sessions(&state);

    // Then the session entry is idle.
    assert_eq!(sessions.len(), 1);
    assert!(
        sessions[0].is_idle,
        "idle session with no busy ops should be idle"
    );
}

#[rstest::rstest]
fn working_complete_returns_to_idle() {
    // Given a session that was working but completed.
    let mut state = AppState::default();
    state.active_session_mut().begin_busy();
    state.active_session_mut().complete_busy();

    // When collecting sorted open sessions.
    let sessions = sorted_open_sessions(&state);

    // Then the session entry is idle again.
    assert_eq!(sessions.len(), 1);
    assert!(sessions[0].is_idle);
}

// --- Rendering ---

use jinn_testutil::setup_term;

fn render_rows(
    section: &mut SessionsSection,
    state: &AppState,
    width: u16,
    height: u16,
) -> Vec<String> {
    let (mut terminal, area) = setup_term(width, height);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            section.render(frame, area, &ctx);
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
            let ctx = RenderCtx::new(&state);
            section.render(frame, area, &ctx);
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

#[rstest::rstest]
fn render_footer_uses_focus_accent_when_sidebar_focused() {
    // Given a sessions section with sidebar focused.
    let mut section = SessionsSection::new();
    let state = {
        let mut s = AppState::default();
        s.frontend.scope_stack.push(FocusScope::SidebarSessions);
        s
    };

    // When rendering.
    let (mut terminal, area) = setup_term(30, 5);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            section.render(frame, area, &ctx);
        })
        .unwrap();

    // Then the footer box-drawing corner (╰) at column 0, row 1 has focus_accent color.
    let buffer = terminal.backend().buffer();
    let corner_cell = buffer.cell((0, 1)).expect("corner cell should exist");
    assert_eq!(corner_cell.symbol(), "\u{2570}");
    assert_eq!(
        corner_cell.style().fg,
        Some(state.frontend.theme.focus_accent)
    );
}

#[rstest::rstest]
fn render_footer_uses_border_unfocused_when_sidebar_not_focused() {
    // Given a sessions section with default state (no sidebar focus).
    let mut section = SessionsSection::new();
    let state = AppState::default();

    // When rendering.
    let (mut terminal, area) = setup_term(30, 5);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            section.render(frame, area, &ctx);
        })
        .unwrap();

    // Then the footer box-drawing corner (╰) at column 0, row 1 has border_unfocused color.
    let buffer = terminal.backend().buffer();
    let corner_cell = buffer.cell((0, 1)).expect("corner cell should exist");
    assert_eq!(corner_cell.symbol(), "\u{2570}");
    assert_eq!(
        corner_cell.style().fg,
        Some(state.frontend.theme.border_unfocused)
    );
}

#[rstest::rstest]
fn render_footer_uses_border_unfocused_when_other_sidebar_section_focused() {
    // Given a sessions section rendered while persona section is focused (not sessions).
    let mut section = SessionsSection::new();
    let state = {
        let mut s = AppState::default();
        s.frontend.scope_stack.push(FocusScope::SidebarPersona);
        s
    };

    // When rendering.
    let (mut terminal, area) = setup_term(30, 5);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            section.render(frame, area, &ctx);
        })
        .unwrap();

    // Then the footer box-drawing corner uses border_unfocused (not focus_accent).
    let buffer = terminal.backend().buffer();
    let corner_cell = buffer.cell((0, 1)).expect("corner cell should exist");
    assert_eq!(corner_cell.symbol(), "\u{2570}");
    assert_eq!(
        corner_cell.style().fg,
        Some(state.frontend.theme.border_unfocused)
    );
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
    assert!(!state.session.contains(&closing_id));
    assert_eq!(state.session.session_count(), 2);
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
    assert!(!state.session.contains(&closing_id));
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
    assert_eq!(state.session.session_count(), 1);
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
    assert!(selected.unwrap() < state.session.session_count());
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
fn close_session_rejected_when_working_phase() {
    // Given state with a session in Working phase.
    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::SidebarSessions);
    state.frontend.sessions_section.selected_index = Some(0);
    state.active_session_mut().begin_busy();

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
            let ctx = RenderCtx::new(&state);
            section.render(frame, area, &ctx);
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
            let ctx = RenderCtx::new(&state);
            section.render(frame, area, &ctx);
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
    assert!(entry_last_is_error(&state, &sessions[0].id));
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
    assert!(!entry_last_is_error(&state, &sessions[0].id));
}

#[rstest::rstest]
fn sorted_sessions_empty_history_is_not_error() {
    // Given a session with no history entries.
    let state = AppState::default();

    // When collecting sorted sessions.
    let sessions = sorted_open_sessions(&state);

    // Then the entry has last_entry_is_error = false.
    assert!(!entry_last_is_error(&state, &sessions[0].id));
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

// --- SessionNewWithLifecycle ---

#[rstest::rstest]
fn session_new_with_lifecycle_opens_picker_from_normal_mode() {
    // Given default app state (Normal mode).
    let mut state = AppState::default();

    // When handling the intent via IntentHandler.
    let result = crate::feat::intent::IntentHandler::handle(
        &crate::Intent::SessionNewWithLifecycle,
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
fn session_new_with_lifecycle_opens_picker_from_sidebar_sessions() {
    // Given sidebar focused on sessions section.
    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::SidebarSessions);

    // When handling the intent via IntentHandler.
    let result = crate::feat::intent::IntentHandler::handle(
        &crate::Intent::SessionNewWithLifecycle,
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
            setup: Some(
                crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(
                    "echo setup".to_owned(),
                ),
            ),
            teardown: Some(
                crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(
                    "cleanup.sh $1".to_owned(),
                ),
            ),
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
            setup: Some(
                crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(
                    "echo setup".to_owned(),
                ),
            ),
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
    state.session.insert(root_a);

    // Create child_a1 under root_a.
    let mut child_a1 = ChatSessionState::new();
    child_a1.set_title("child a1".to_owned());
    child_a1.set_parent_session(root_a_id.clone());
    let child_a1_id = child_a1.session_id().clone();
    state.session.insert(child_a1);

    // Create grandchild_a1a under child_a1.
    let mut grandchild = ChatSessionState::new();
    grandchild.set_title("grandchild a1a".to_owned());
    grandchild.set_parent_session(child_a1_id.clone());
    let _grandchild_id = grandchild.session_id().clone();
    state.session.insert(grandchild);

    // Create child_a2 under root_a.
    let mut child_a2 = ChatSessionState::new();
    child_a2.set_title("child a2".to_owned());
    child_a2.set_parent_session(root_a_id.clone());
    let _child_a2_id = child_a2.session_id().clone();
    state.session.insert(child_a2);

    // Create root_b with a title (newest root).
    let mut newest_root = ChatSessionState::new();
    newest_root.push_entry(ChatEntry::user("root b"));
    newest_root.set_title("root b".to_owned());
    let newest_root_id = newest_root.session_id().clone();
    state.session.insert(newest_root);

    // Remove the default session (created at AppState::default).
    let default_id = state.session.active_session_id().clone();
    if default_id != root_a_id && default_id != newest_root_id {
        state.session.remove(&default_id);
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
        entry_created_at(&state, &roots[0].id) >= entry_created_at(&state, &roots[1].id),
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
        .find(|s| entry_title(&state, &s.id).contains("root a"))
        .map(|s| s.id.clone())
        .expect("root a should exist");
    let children: Vec<_> = sessions
        .iter()
        .filter(|s| s.parent_id.as_ref() == Some(&root_a_id))
        .collect();

    // Then children are sorted oldest-first.
    assert_eq!(children.len(), 2, "root_a should have 2 children");
    assert!(
        entry_created_at(&state, &children[0].id) <= entry_created_at(&state, &children[1].id),
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
        .position(|s| entry_title(&state, &s.id).contains("root a"))
        .expect("root a");
    let child_a1_pos = sessions
        .iter()
        .position(|s| entry_title(&state, &s.id).contains("child a1"))
        .expect("child a1");
    let grandchild_pos = sessions
        .iter()
        .position(|s| entry_title(&state, &s.id).contains("grandchild"))
        .expect("grandchild");
    let child_a2_pos = sessions
        .iter()
        .position(|s| entry_title(&state, &s.id).contains("child a2"))
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
    state.session.insert(orphan);

    // When collecting sorted sessions.
    let sessions = sorted_open_sessions(&state);

    // Then the orphan appears as a root (depth 0).
    let orphan_entry = sessions
        .iter()
        .find(|s| entry_title(&state, &s.id).contains("orphan"));
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
        .position(|s| entry_title(&state, &s.id).contains("root a"))
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
        entry_title(&state, &new_sessions[new_index].id).contains("child a1"),
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
        .position(|s| entry_title(&state, &s.id).contains("child a1"))
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
        entry_title(&state, &new_sessions[new_index].id).contains("root a"),
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
        .position(|s| entry_title(&state, &s.id).contains("child a1"))
        .expect("child a1");
    let child_a1_id = sessions[child_a1_index].id.clone();
    state.frontend.sessions_section.selected_index = Some(child_a1_index);

    // When closing child_a1.
    handle_session_close(&mut state);

    // Then child_a1 is removed.
    assert!(!state.session.contains(&child_a1_id));
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
        .position(|s| entry_title(&state, &s.id).contains("root a"))
        .expect("root a");
    let root_a_id = sessions[root_a_index].id.clone();
    state.frontend.sessions_section.selected_index = Some(root_a_index);

    // When closing root_a.
    handle_session_close(&mut state);

    // Then root_a is removed.
    assert!(!state.session.contains(&root_a_id));
    // And its former children are now orphans (roots in the new tree).
    let remaining = sorted_open_sessions(&state);
    let former_children: Vec<_> = remaining
        .iter()
        .filter(|s| {
            entry_title(&state, &s.id).contains("child a")
                || entry_title(&state, &s.id).contains("grandchild")
        })
        .collect();
    assert!(
        !former_children.is_empty(),
        "former children should still be present"
    );
    // All former children should now be roots or have adjusted depth.
    // The children of root_a become orphans → treated as roots.
    let child_a1_entry = remaining
        .iter()
        .find(|s| entry_title(&state, &s.id).contains("child a1"));
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
        .position(|s| entry_title(&state, &s.id).contains("child a1"))
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
    let (mut terminal, area) = jinn_testutil::setup_term(30, 15);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            section.render(frame, area, &ctx);
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

// ---------------------------------------------------------------------------
// Visual reparenting tests
// ---------------------------------------------------------------------------

#[rstest::rstest]
fn archiving_intermediate_parent_reparents_grandchild_under_grandparent() {
    // Given state with root_a -> child_a1 -> grandchild_a1a.
    let mut state = state_with_tree();
    state.frontend.scope_stack.push(FocusScope::SidebarSessions);
    let sessions = sorted_open_sessions(&state);
    let child_a1_index = sessions
        .iter()
        .position(|s| entry_title(&state, &s.id).contains("child a1"))
        .expect("child a1");
    let child_a1_id = sessions[child_a1_index].id.clone();
    let root_a_id = sessions
        .iter()
        .find(|s| entry_title(&state, &s.id).contains("root a"))
        .map(|s| s.id.clone())
        .expect("root a");
    let grandchild_id = sessions
        .iter()
        .find(|s| entry_title(&state, &s.id).contains("grandchild"))
        .map(|s| s.id.clone())
        .expect("grandchild");
    state.frontend.sessions_section.selected_index = Some(child_a1_index);

    // When closing child_a1 (the intermediate parent).
    handle_session_close(&mut state);

    // Then child_a1 is removed.
    assert!(!state.session.contains(&child_a1_id));
    // And the visual_parents index maps grandchild -> root_a.
    assert_eq!(
        state
            .frontend
            .sessions_section
            .visual_parents
            .get(&grandchild_id),
        Some(&root_a_id),
        "grandchild should be reparented to root_a in visual_parents"
    );
    // And sorted_open_sessions shows grandchild at depth 1 under root_a.
    let remaining = sorted_open_sessions(&state);
    let grandchild_entry = remaining
        .iter()
        .find(|s| entry_title(&state, &s.id).contains("grandchild"))
        .expect("grandchild should exist");
    assert_eq!(
        grandchild_entry.depth, 1,
        "grandchild should be at depth 1 under root_a, got depth {}",
        grandchild_entry.depth
    );
    assert_eq!(
        grandchild_entry.parent_id,
        Some(root_a_id),
        "grandchild's effective parent should be root_a"
    );
}

#[rstest::rstest]
fn archiving_root_does_not_create_visual_parents_for_orphaned_children() {
    // Given state with root_a having children.
    let mut state = state_with_tree();
    state.frontend.scope_stack.push(FocusScope::SidebarSessions);
    let sessions = sorted_open_sessions(&state);
    let root_a_index = sessions
        .iter()
        .position(|s| entry_title(&state, &s.id).contains("root a"))
        .expect("root a");
    let _root_a_id = sessions[root_a_index].id.clone();
    state.frontend.sessions_section.selected_index = Some(root_a_index);

    // When closing root_a (no loaded ancestor to reparent to).
    handle_session_close(&mut state);

    // Then the visual_parents index should be empty (root has no loaded ancestor).
    assert!(
        state.frontend.sessions_section.visual_parents.is_empty(),
        "no visual_parents entries should exist when root is closed"
    );
}

#[rstest::rstest]
fn multi_level_intermediate_hiding_reparents_to_nearest_loaded_ancestor() {
    // Given a chain: root -> A -> B -> leaf.
    let mut state = AppState::default();

    let mut root = ChatSessionState::new();
    root.set_title("root".to_owned());
    let root_id = root.session_id().clone();
    state.session.insert(root);

    let mut a = ChatSessionState::new();
    a.set_title("session A".to_owned());
    a.set_parent_session(root_id.clone());
    let a_id = a.session_id().clone();
    state.session.insert(a);

    let mut b = ChatSessionState::new();
    b.set_title("session B".to_owned());
    b.set_parent_session(a_id.clone());
    let b_id = b.session_id().clone();
    state.session.insert(b);

    let mut leaf = ChatSessionState::new();
    leaf.set_title("leaf".to_owned());
    leaf.set_parent_session(b_id.clone());
    let leaf_id = leaf.session_id().clone();
    state.session.insert(leaf);

    // Remove default session.
    let default_id = state.session.active_session_id().clone();
    if default_id != root_id {
        state.session.remove(&default_id);
    }
    state.session.set_active(root_id.clone());

    // When archiving A.
    state.frontend.scope_stack.push(FocusScope::SidebarSessions);
    let sessions = sorted_open_sessions(&state);
    let a_index = sessions.iter().position(|s| s.id == a_id).expect("A");
    state.frontend.sessions_section.selected_index = Some(a_index);
    handle_session_close(&mut state);

    // Then B is reparented to root.
    assert_eq!(
        state.frontend.sessions_section.visual_parents.get(&b_id),
        Some(&root_id),
        "B should be reparented to root"
    );

    // When archiving B.
    let sessions = sorted_open_sessions(&state);
    let b_index = sessions.iter().position(|s| s.id == b_id).expect("B");
    state.frontend.sessions_section.selected_index = Some(b_index);
    handle_session_close(&mut state);

    // Then leaf is reparented to root (transitive via B visual parent).
    assert_eq!(
        state.frontend.sessions_section.visual_parents.get(&leaf_id),
        Some(&root_id),
        "leaf should be reparented to root (transitive)"
    );

    // And the sidebar tree shows leaf at depth 1 under root.
    let remaining = sorted_open_sessions(&state);
    let leaf_entry = remaining.iter().find(|s| s.id == leaf_id).expect("leaf");
    assert_eq!(
        leaf_entry.depth, 1,
        "leaf should be at depth 1 under root, got depth {}",
        leaf_entry.depth
    );
}

// --- sorted_open_sessions: is_last_child for roots (kills == -> != and - -> +// mutants on line 165) ---

#[rstest::rstest]
fn sorted_sessions_last_root_is_marked_as_last_child() {
    // Given 3 root sessions (no parent-child relationships).
    let state = state_with_sessions(3);

    // When collecting sorted sessions.
    let sessions = sorted_open_sessions(&state);

    // Then the last root is marked as is_last_child, others are not.
    assert_eq!(sessions.len(), 3, "should have 3 sessions");
    assert!(
        !sessions[0].is_last_child,
        "first root should not be last child"
    );
    assert!(
        !sessions[1].is_last_child,
        "second root should not be last child"
    );
    assert!(sessions[2].is_last_child, "last root should be last child");
}

#[rstest::rstest]
fn sorted_sessions_single_root_is_marked_as_last_child() {
    // Given a single root session.
    let state = state_with_sessions(1);

    // When collecting sorted sessions.
    let sessions = sorted_open_sessions(&state);

    // Then it is marked as last child.
    assert_eq!(sessions.len(), 1);
    assert!(
        sessions[0].is_last_child,
        "single root should be last child"
    );
}

// --- dfs_children: is_last_child for non-root entries (kills == -> !=
// and - -> +/- // mutants on line 308) ---

#[rstest::rstest]
fn tree_children_last_child_flag_is_correct() {
    // Given state with root_a having two children.
    let state = state_with_tree();

    // When collecting sorted sessions.
    let sessions = sorted_open_sessions(&state);

    // Find root_a's children.
    let root_a_id = sessions
        .iter()
        .find(|s| entry_title(&state, &s.id).contains("root a"))
        .map(|s| s.id.clone())
        .expect("root a should exist");
    let children: Vec<_> = sessions
        .iter()
        .filter(|s| s.parent_id.as_ref() == Some(&root_a_id))
        .collect();

    assert_eq!(children.len(), 2, "root_a should have 2 children");
    assert!(
        !children[0].is_last_child,
        "first child should not be last child"
    );
    assert!(
        children[1].is_last_child,
        "second child should be last child"
    );
}

// --- update_visual_parents_on_removal: kills == -> != mutant on line 250 ---

#[rstest::rstest]
fn update_visual_parents_on_removal_reparents_only_children_of_removed_session() {
    // Given a chain: root -> A -> B.
    use crate::feat::ui::sidebar::sessions::update_visual_parents_on_removal;

    let mut state = AppState::default();

    let mut root = ChatSessionState::new();
    root.set_title("root".to_owned());
    let root_id = root.session_id().clone();
    state.session.insert(root);

    let mut a = ChatSessionState::new();
    a.set_title("session A".to_owned());
    a.set_parent_session(root_id.clone());
    let a_id = a.session_id().clone();
    state.session.insert(a);

    let mut b = ChatSessionState::new();
    b.set_title("session B".to_owned());
    b.set_parent_session(a_id.clone());
    let b_id = b.session_id().clone();
    state.session.insert(b);

    // Also add an unrelated session with its own visual parent.
    let mut unrelated_parent = ChatSessionState::new();
    unrelated_parent.set_title("unrelated parent".to_owned());
    let unrelated_parent_id = unrelated_parent.session_id().clone();
    state.session.insert(unrelated_parent);

    let mut unrelated_child = ChatSessionState::new();
    unrelated_child.set_title("unrelated child".to_owned());
    unrelated_child.set_parent_session(unrelated_parent_id.clone());
    let unrelated_child_id = unrelated_child.session_id().clone();
    state.session.insert(unrelated_child);

    // Remove default session.
    let default_id = state.session.active_session_id().clone();
    state.session.remove(&default_id);
    state.session.set_active(root_id.clone());

    // When removing A.
    update_visual_parents_on_removal(&mut state, &a_id);

    // Then B is reparented to root.
    assert_eq!(
        state.frontend.sessions_section.visual_parents.get(&b_id),
        Some(&root_id),
        "B should be reparented to root"
    );
    // And the unrelated child is NOT reparented (it has a different parent).
    assert_eq!(
        state
            .frontend
            .sessions_section
            .visual_parents
            .get(&unrelated_child_id),
        None,
        "unrelated child should not be reparented - its parent is not being removed"
    );
}

// --- clear_visual_parents_on_load: kills != -> == mutant on line 279 ---

#[rstest::rstest]
fn clear_visual_parents_on_load_removes_only_entries_pointing_to_loaded_session() {
    // Given a state with visual_parents entries.
    use crate::feat::ui::sidebar::sessions::clear_visual_parents_on_load;

    let mut state = AppState::default();
    let id_x = crate::protocol::SessionId::new();
    let id_y = crate::protocol::SessionId::new();
    let loaded_id = crate::protocol::SessionId::new();
    let other_id = crate::protocol::SessionId::new();

    // entry_x -> loaded_id (should be removed after load)
    state
        .frontend
        .sessions_section
        .visual_parents
        .insert(id_x.clone(), loaded_id.clone());
    // entry_y -> other_id (should be kept)
    state
        .frontend
        .sessions_section
        .visual_parents
        .insert(id_y.clone(), other_id.clone());

    // When clearing on load for loaded_id.
    clear_visual_parents_on_load(&mut state, &loaded_id);

    // Then only entry_x is removed (pointed to loaded_id).
    assert_eq!(
        state.frontend.sessions_section.visual_parents.get(&id_x),
        None,
        "entry pointing to loaded session should be removed"
    );
    assert_eq!(
        state.frontend.sessions_section.visual_parents.get(&id_y),
        Some(&other_id),
        "entry pointing to other session should be kept"
    );
}

// --- clear_visual_parents_on_load: kills replace fn with () mutant ---

#[rstest::rstest]
fn clear_visual_parents_on_load_actually_removes_entries() {
    // Given a state with a visual_parents entry that should be cleared.
    use crate::feat::ui::sidebar::sessions::clear_visual_parents_on_load;

    let mut state = AppState::default();
    let child_id = crate::protocol::SessionId::new();
    let loaded_id = crate::protocol::SessionId::new();

    state
        .frontend
        .sessions_section
        .visual_parents
        .insert(child_id, loaded_id.clone());
    assert_eq!(state.frontend.sessions_section.visual_parents.len(), 1);

    // When clearing on load.
    clear_visual_parents_on_load(&mut state, &loaded_id);

    // Then the entry is removed.
    assert!(
        state.frontend.sessions_section.visual_parents.is_empty(),
        "visual_parents should be empty after clearing the loaded session's entries"
    );
}

// --- Workflow entries in sorted_open_sessions ---

fn state_with_workflows() -> AppState {
    use crate::feat::workflow::attached_workflow::{
        AttachedWorkflow, WorkflowConfig, WorkflowTrigger,
    };

    let mut state = AppState::default();

    // Create root session.
    let mut root = ChatSessionState::new();
    root.set_title("main session".to_owned());
    let root_id = root.session_id().clone();
    state.session.insert(root);

    // Create a child session under root.
    let mut child = ChatSessionState::new();
    child.set_title("child session".to_owned());
    child.set_parent_session(root_id.clone());
    let _child_id = child.session_id().clone();
    state.session.insert(child);

    // Remove the default session if different from root.
    let default_id = state.session.active_session_id().clone();
    if default_id != root_id {
        state.session.remove(&default_id);
    }
    state.session.set_active(root_id.clone());

    // Attach two workflows to root session.
    let root_session = state.session.get_mut(&root_id).expect("root session");
    root_session
        .core
        .attached_workflows
        .push(AttachedWorkflow::new(
            WorkflowConfig {
                script: "consensus".to_owned(),
                data: serde_json::json!({}),
            },
            WorkflowTrigger::TurnEnd,
        ));
    root_session
        .core
        .attached_workflows
        .push(AttachedWorkflow::new(
            WorkflowConfig {
                script: "judge_fail".to_owned(),
                data: serde_json::json!({}),
            },
            WorkflowTrigger::Manual,
        ));

    state
}

#[rstest::rstest]
fn sorted_open_sessions_includes_attached_workflows() {
    // Given a session with two attached workflows.
    let state = state_with_workflows();

    // When collecting sorted sessions.
    let sessions = sorted_open_sessions(&state);

    // Then the session entry appears, followed by its child session, then two workflow entries.
    let titles: Vec<&str> = sessions.iter().map(|s| s.title.as_str()).collect();
    assert!(
        titles.contains(&"main session"),
        "root session should be present"
    );
    assert!(
        titles.contains(&"child session"),
        "child session should be present"
    );
    assert!(
        titles.contains(&"consensus"),
        "consensus workflow should be present"
    );
    assert!(
        titles.contains(&"judge_fail"),
        "judge_fail workflow should be present"
    );

    // And workflow entries have the Workflow kind.
    let workflow_entries: Vec<_> = sessions
        .iter()
        .filter(|s| matches!(s.kind, SessionEntryKind::Workflow { .. }))
        .collect();
    assert_eq!(workflow_entries.len(), 2, "should have 2 workflow entries");

    // And workflow entries are children of root session (depth = root depth + 1).
    let root_entry = sessions
        .iter()
        .find(|s| s.title == "main session")
        .expect("root");
    let root_depth = root_entry.depth;
    for wf in &workflow_entries {
        assert_eq!(
            wf.depth,
            root_depth + 1,
            "workflow should be one level deeper than its parent session"
        );
        assert_eq!(
            wf.parent_id,
            Some(root_entry.id.clone()),
            "workflow parent should be root session"
        );
    }
}

#[rstest::rstest]
fn sorted_open_sessions_no_workflows_when_none_attached() {
    // Given a session with no attached workflows.
    let state = AppState::default();

    // When collecting sorted sessions.
    let sessions = sorted_open_sessions(&state);

    // Then there are no workflow entries.
    let workflow_count = sessions
        .iter()
        .filter(|s| matches!(s.kind, SessionEntryKind::Workflow { .. }))
        .count();
    assert_eq!(workflow_count, 0, "should have no workflow entries");
}

#[rstest::rstest]
fn sorted_open_sessions_workflows_after_real_children() {
    // Given a session with a child session and two attached workflows.
    let state = state_with_workflows();

    // When collecting sorted sessions.
    let sessions = sorted_open_sessions(&state);

    // Then child session appears before workflow entries.
    let child_pos = sessions
        .iter()
        .position(|s| s.title == "child session")
        .expect("child");
    let wf1_pos = sessions
        .iter()
        .position(|s| s.title == "consensus")
        .expect("consensus");
    let wf2_pos = sessions
        .iter()
        .position(|s| s.title == "judge_fail")
        .expect("judge_fail");
    assert!(
        child_pos < wf1_pos,
        "child session should appear before workflow entries"
    );
    assert!(wf1_pos < wf2_pos, "workflows should appear in order");
}

// --- Workflow entry navigation and activation ---

fn state_with_session_and_workflows(workflow_count: usize) -> AppState {
    use crate::feat::workflow::attached_workflow::{
        AttachedWorkflow, WorkflowConfig, WorkflowTrigger,
    };

    let mut state = AppState::default();
    let session = state.session.active_session_id().clone();
    {
        let s = state.session.get_mut(&session).expect("active session");
        for i in 0..workflow_count {
            let aw = AttachedWorkflow::new(
                WorkflowConfig {
                    script: format!("plugin-{i}"),
                    data: serde_json::json!({}),
                },
                WorkflowTrigger::TurnEnd,
            );
            s.core.attached_workflows.push(aw);
        }
    }
    state
}

#[test]
fn navigate_down_skips_workflow_entries() {
    // Given a session with 2 workflow attachments, cursor at index 0.
    let mut state = state_with_session_and_workflows(2);
    state.frontend.sessions_section.selected_index = Some(0);

    // When navigating down past the workflow entries.
    // entries: [session0, wf-0, wf-1]
    // pressing down from session0 should skip wf-0 and wf-1 and return Exhausted.
    let result = navigate(&SidebarIntent::MoveDown, &mut state);

    // Then navigation returns Exhausted (no next session).
    assert_eq!(result, SectionNavResult::Exhausted);
}

#[test]
fn navigate_up_skips_workflow_entries() {
    // Given two sessions, first with a workflow, cursor at second session.
    use crate::feat::workflow::attached_workflow::{
        AttachedWorkflow, WorkflowConfig, WorkflowTrigger,
    };
    let mut state = state_with_sessions(2);
    {
        let sessions: Vec<_> = state.session.iter().collect();
        let first_id = sessions[0].0.clone();
        let s = state.session.get_mut(&first_id).expect("first session");
        let aw = AttachedWorkflow::new(
            WorkflowConfig {
                script: "my-plugin".to_owned(),
                data: serde_json::json!({}),
            },
            WorkflowTrigger::TurnEnd,
        );
        s.core.attached_workflows.push(aw);
    }
    // entries: [session0, wf, session1]
    // cursor on session1
    let entries = sorted_open_sessions(&state);
    let session1_idx = entries
        .iter()
        .rposition(|e| matches!(e.kind, SessionEntryKind::Session))
        .expect("session1");
    state.frontend.sessions_section.selected_index = Some(session1_idx);

    // When navigating up from session1.
    let result = navigate(&SidebarIntent::MoveUp, &mut state);

    // Then cursor lands on session0 (skipping the workflow entry).
    assert_eq!(result, SectionNavResult::Moved);
    assert_eq!(state.frontend.sessions_section.selected_index, Some(0));
}

#[test]
fn activate_on_workflow_entry_is_noop() {
    // Given a session with a workflow attachment, cursor on the workflow entry.
    let mut state = state_with_session_and_workflows(1);
    let original_active = state.session.active_session_id().clone();
    // entries: [session0, wf-0]
    state.frontend.sessions_section.selected_index = Some(1);

    // When activating the workflow entry.
    handle_session_activate(&mut state);

    // Then the active session did not change.
    assert_eq!(*state.session.active_session_id(), original_active);
}

#[test]
fn close_on_workflow_entry_is_rejected() {
    // Given a session with a workflow attachment, cursor on the workflow entry.
    let mut state = state_with_session_and_workflows(1);
    // entries: [session0, wf-0]
    state.frontend.sessions_section.selected_index = Some(1);
    state.frontend.close_session_prompt = true;

    // When attempting to close the workflow entry.
    let result = validate_session_close(&state);

    // Then validation rejects it.
    assert!(result.is_err());
}
