//! Tests for the session preview popup renderer.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::theme::default_theme;
use crate::feat::ui::sidebar::sessions::preview::{
    render_session_preview, session_preview_popup_rect,
};
use crate::protocol::ChatEntry;
use nullslop_testutil::{buffer_row, setup_term};
use ratatui::layout::Rect;

fn make_session_with_entries(n: usize) -> ChatSessionState {
    let mut session = ChatSessionState::new();
    for i in 0..n {
        session.push_entry(ChatEntry::user(format!("message {i}")));
    }
    session
}

fn make_session_with_title(title: &str) -> ChatSessionState {
    let mut session = ChatSessionState::new();
    session.set_title(title.to_owned());
    session
}

fn render_preview(
    session: &ChatSessionState,
    term_width: u16,
    term_height: u16,
) -> (ratatui::buffer::Buffer, Rect) {
    let theme = default_theme();
    let frame_area = Rect::new(0, 0, term_width, term_height);
    // Simulate sessions section starting at row 20.
    let sessions_top_y = 20u16;
    let popup_area = session_preview_popup_rect(frame_area, sessions_top_y, 20);

    let (mut terminal, _) = setup_term(term_width, term_height);
    terminal
        .draw(|frame| {
            render_session_preview(frame, popup_area, session, &theme, None);
        })
        .unwrap();

    let buffer = terminal.backend().buffer().clone();
    (buffer, popup_area)
}

#[rstest::rstest]
fn empty_history_shows_title_and_keybinds_only() {
    // Given a session with no entries and a title.
    let session = make_session_with_title("Empty Session");

    // When rendering the preview.
    let (buffer, popup_area) = render_preview(&session, 80, 24);

    // Then the popup title appears in the top border.
    let top_row = buffer_row(&buffer, popup_area.y, popup_area.x + popup_area.width);
    assert!(
        top_row.contains("Empty Session"),
        "popup title should contain 'Empty Session', got: {top_row}"
    );

    // And the keybinds bar appears in the bottom of the popup.
    let bottom_inner_y = popup_area.y + popup_area.height - 2;
    let bar_row = buffer_row(&buffer, bottom_inner_y, popup_area.x + popup_area.width);
    assert!(
        bar_row.contains('c') || bar_row.contains('r') || bar_row.contains('x'),
        "keybinds bar should contain key hints, got: {bar_row}"
    );
}

#[rstest::rstest]
fn last_five_entries_rendered() {
    // Given a session with 8 entries.
    let session = make_session_with_entries(8);

    // When rendering the preview.
    let (buffer, popup_area) = render_preview(&session, 80, 24);

    // Then the content shows entries from index 3 onward (last 5).
    // Entry "message 3" through "message 7" should be visible.
    let content_start_y = popup_area.y + 1;
    let content_end_y = popup_area.y + popup_area.height - 2;
    let mut all_text = String::new();
    for y in content_start_y..=content_end_y {
        all_text.push_str(&buffer_row(&buffer, y, popup_area.x + popup_area.width));
    }
    assert!(
        all_text.contains("message 7"),
        "should contain the last entry 'message 7', got text: {all_text}"
    );
    assert!(
        all_text.contains("message 3"),
        "should contain 'message 3' (5th from end), got text: {all_text}"
    );
    assert!(
        !all_text.contains("message 2"),
        "should NOT contain 'message 2' (6th from end), got text: {all_text}"
    );
}

#[rstest::rstest]
fn lines_truncated_to_twenty() {
    // Given a session with entries that produce many lines.
    let mut session = ChatSessionState::new();
    // 5 entries with 10 newlines each = 50+ lines total (plus padding).
    for i in 0..5 {
        let text = (0..10)
            .map(|j| format!("line {i}-{j}"))
            .collect::<Vec<_>>()
            .join("\n");
        session.push_entry(ChatEntry::assistant(text));
    }

    // When rendering the preview.
    let (_buffer, popup_area) = render_preview(&session, 80, 30);

    // Then the content area does not exceed the available height.
    // The popup should have been capped to fit within 30 - 4 = 26 rows max.
    assert!(
        popup_area.height <= 26,
        "popup height should be capped, got: {}",
        popup_area.height
    );
}

#[rstest::rstest]
fn keybinds_bar_contains_all_four_keybinds() {
    // Given a session with one entry.
    let session = make_session_with_entries(1);

    // When rendering the preview.
    let (buffer, popup_area) = render_preview(&session, 80, 24);

    // Then the keybinds bar (bottom inner row) contains all four keybind letters.
    let bar_y = popup_area.y + popup_area.height - 2;
    let bar_row = buffer_row(&buffer, bar_y, popup_area.x + popup_area.width);

    assert!(
        bar_row.contains('c'),
        "bar should contain 'c' keybind, got: {bar_row}"
    );
    assert!(
        bar_row.contains('r'),
        "bar should contain 'r' keybind, got: {bar_row}"
    );
    assert!(
        bar_row.contains('x'),
        "bar should contain 'x' keybind, got: {bar_row}"
    );
    assert!(
        bar_row.contains('a'),
        "bar should contain 'a' keybind, got: {bar_row}"
    );
}

#[rstest::rstest]
fn popup_title_shows_session_title() {
    // Given a session with a custom title.
    let session = make_session_with_title("My Custom Session");

    // When rendering the preview.
    let (buffer, popup_area) = render_preview(&session, 80, 24);

    // Then the top border row contains the session title.
    let top_row = buffer_row(&buffer, popup_area.y, popup_area.x + popup_area.width);
    assert!(
        top_row.contains("My Custom Session"),
        "popup top border should contain 'My Custom Session', got: {top_row}"
    );
}
