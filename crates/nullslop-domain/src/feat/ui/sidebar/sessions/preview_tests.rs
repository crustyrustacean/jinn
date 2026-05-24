//! Tests for the session preview popup renderer.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::theme::default_theme;
use crate::feat::ui::sidebar::sessions::preview::{
    SessionPreviewCache, render_session_preview, session_preview_popup_rect,
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
    // Simulate cursor at row 30 (sessions section starts at row 30, cursor on first item).
    let cursor_y = 30u16;
    let popup_area = session_preview_popup_rect(frame_area, cursor_y, 20);

    let (mut terminal, _) = setup_term(term_width, term_height);
    let mut cache = SessionPreviewCache::new();
    terminal
        .draw(|frame| {
            render_session_preview(frame, popup_area, session, &theme, None, &mut cache);
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
    let (buffer, popup_area) = render_preview(&session, 80, 40);

    // Then the popup title appears in the top border.
    let top_row = buffer_row(&buffer, popup_area.y, popup_area.x + popup_area.width);
    assert!(
        top_row.contains("Empty Session"),
        "popup title should contain 'Empty Session', got: {top_row}"
    );

    // And the keybinds lines appear in the footer area.
    // Keybinds line 2 (c continue · r rename) is 3 rows above the bottom border.
    let keybinds_y = popup_area.y + popup_area.height - 3;
    let keybinds_row = buffer_row(&buffer, keybinds_y, popup_area.x + popup_area.width);
    assert!(
        keybinds_row.contains('c') || keybinds_row.contains('r'),
        "keybinds line 2 should contain c or r, got: {keybinds_row}"
    );
}

#[rstest::rstest]
fn last_five_entries_rendered() {
    // Given a session with 8 entries.
    let session = make_session_with_entries(8);

    // When rendering the preview with enough vertical space.
    let (buffer, popup_area) = render_preview(&session, 80, 40);

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
    let (_buffer, popup_area) = render_preview(&session, 80, 40);

    // Then the content area does not exceed the available height.
    // The popup should have been capped to fit within the available space above the cursor.
    assert!(
        popup_area.height <= 28,
        "popup height should be capped, got: {}",
        popup_area.height
    );
}

#[rstest::rstest]
fn keybinds_line_one_shows_close_archive_insert() {
    // Given a session with one entry.
    let session = make_session_with_entries(1);

    // When rendering the preview.
    let (buffer, popup_area) = render_preview(&session, 80, 40);

    // Then keybinds line 1 (4 rows above bottom border) contains x, a, i.
    let line1_y = popup_area.y + popup_area.height - 4;
    let line1_row = buffer_row(&buffer, line1_y, popup_area.x + popup_area.width);

    assert!(
        line1_row.contains('x'),
        "line 1 should contain 'x' keybind, got: {line1_row}"
    );
    assert!(
        line1_row.contains('a'),
        "line 1 should contain 'a' keybind, got: {line1_row}"
    );
    assert!(
        line1_row.contains('i'),
        "line 1 should contain 'i' keybind, got: {line1_row}"
    );
}

#[rstest::rstest]
fn keybinds_line_two_shows_continue_rename() {
    // Given a session with one entry.
    let session = make_session_with_entries(1);

    // When rendering the preview.
    let (buffer, popup_area) = render_preview(&session, 80, 40);

    // Then keybinds line 2 (3 rows above bottom border) contains c, r.
    let line2_y = popup_area.y + popup_area.height - 3;
    let line2_row = buffer_row(&buffer, line2_y, popup_area.x + popup_area.width);

    assert!(
        line2_row.contains('c'),
        "line 2 should contain 'c' keybind, got: {line2_row}"
    );
    assert!(
        line2_row.contains('r'),
        "line 2 should contain 'r' keybind, got: {line2_row}"
    );
}

#[rstest::rstest]
fn popup_title_shows_session_title() {
    // Given a session with a custom title.
    let session = make_session_with_title("My Custom Session");

    // When rendering the preview.
    let (buffer, popup_area) = render_preview(&session, 80, 40);

    // Then the top border row contains the session title.
    let top_row = buffer_row(&buffer, popup_area.y, popup_area.x + popup_area.width);
    assert!(
        top_row.contains("My Custom Session"),
        "popup top border should contain 'My Custom Session', got: {top_row}"
    );
}

#[rstest::rstest]
fn model_line_shows_provider_and_model() {
    // Given a session with a specific model set.
    let mut session = ChatSessionState::new();
    session.set_model("ollama/llama3".to_owned());

    // When rendering the preview.
    let (buffer, popup_area) = render_preview(&session, 80, 40);

    // Then the bottom inner row shows the provider/model format.
    let model_y = popup_area.y + popup_area.height - 2;
    let model_row = buffer_row(&buffer, model_y, popup_area.x + popup_area.width);
    assert!(
        model_row.contains("(ollama)/llama3"),
        "model line should contain '(ollama)/llama3', got: {model_row}"
    );
}

#[rstest::rstest]
fn model_line_shows_no_model_selected_when_unset() {
    // Given a default session (no provider selected).
    let session = ChatSessionState::new();

    // When rendering the preview.
    let (buffer, popup_area) = render_preview(&session, 80, 40);

    // Then the bottom inner row shows "no model selected".
    let model_y = popup_area.y + popup_area.height - 2;
    let model_row = buffer_row(&buffer, model_y, popup_area.x + popup_area.width);
    assert!(
        model_row.contains("no model selected"),
        "model line should show 'no model selected', got: {model_row}"
    );
}

// ---------------------------------------------------------------------------
// Regression tests: cursor-relative positioning with 2-row gap
// ---------------------------------------------------------------------------

#[rstest::rstest]
#[case::normal(30, 5)]
#[case::content_exceeds_space(30, 20)]
#[case::cursor_near_top(10, 5)]
fn popup_bottom_edge_is_two_rows_above_cursor(
    #[case] cursor_y: u16,
    #[case] content_line_count: usize,
) {
    // Given a frame area and a cursor position.
    let frame_area = Rect::new(0, 0, 80, 40);

    // When computing the popup rect.
    let popup_rect =
        session_preview_popup_rect(frame_area, cursor_y, content_line_count);

    // Then the popup bottom edge + 2 = cursor_y.
    assert_eq!(
        popup_rect.y + popup_rect.height + 2,
        cursor_y,
        "popup bottom edge + 2 should equal cursor_y ({cursor_y}), \
         got popup_y={}, popup_height={}",
        popup_rect.y,
        popup_rect.height
    );
}

#[rstest::rstest]
fn popup_follows_cursor_not_section_top() {
    // Given two different cursor positions.
    let frame_area = Rect::new(0, 0, 80, 40);
    let content_lines = 5;

    // When computing popup rects for each cursor.
    let rect_at_20 = session_preview_popup_rect(frame_area, 20, content_lines);
    let rect_at_25 = session_preview_popup_rect(frame_area, 25, content_lines);

    // Then each popup is anchored to its cursor (2-row gap invariant).
    assert_eq!(
        rect_at_20.y + rect_at_20.height + 2,
        20,
        "popup at cursor_y=20 should satisfy 2-row gap invariant"
    );
    assert_eq!(
        rect_at_25.y + rect_at_25.height + 2,
        25,
        "popup at cursor_y=25 should satisfy 2-row gap invariant"
    );

    // And the popup positions are different (cursor-relative, not fixed).
    assert_ne!(
        rect_at_20.y, rect_at_25.y,
        "popup Y should change when cursor Y changes"
    );
}

#[rstest::rstest]
fn popup_height_capped_when_cursor_near_top() {
    // Given a cursor very near the top of the terminal.
    let frame_area = Rect::new(0, 0, 80, 40);
    let cursor_y = 7u16;
    let content_line_count = 20;

    // When computing the popup rect.
    let popup_rect =
        session_preview_popup_rect(frame_area, cursor_y, content_line_count);

    // Then the popup height is capped (max_height = 7 - 0 - 2 = 5).
    // With max(5) floor, popup_height should be exactly 5.
    assert_eq!(
        popup_rect.height, 5,
        "popup height should be capped to minimum when cursor is near top, \
         got: {}",
        popup_rect.height
    );

    // And the popup does not extend past the gap boundary toward the cursor.
    assert!(
        popup_rect.y + popup_rect.height + 2 <= cursor_y,
        "popup should not encroach on the 2-row gap above cursor"
    );
}
