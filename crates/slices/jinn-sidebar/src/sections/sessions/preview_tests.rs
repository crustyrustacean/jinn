//! Tests for the session preview popup renderer.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::indexing_slicing,
    reason = "test code"
)]

use crate::sections::sessions::preview::{
    DEFAULT_TOOL_ENTRY_MAX_LINES, LOADING_CONTENT_ROWS, render_session_preview,
    render_session_preview_loading, session_preview_popup_rect,
};
use jinn_chat_log_view::chat_log::RenderContext;
use jinn_chat_log_view_msg::{PREVIEW_ENTRY_COUNT, PREVIEW_MAX_LINES};
use jinn_core_types::model_selection::ModelSelection;
use jinn_domain::feat::ui::chat_log::render_preview as render_preview_lines;
use jinn_domain::protocol::ChatEntry;
use jinn_session_state::ChatSessionState;
use jinn_testutil::{buffer_row, setup_term};
use jinn_theme::default_theme;
use jinn_tools_msg::{PhaseInput, TaskStatus};
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

    // The lines a worker would have published for this session, rendered the
    // same way the chat log renders them.
    let lines = {
        let ctx = RenderContext {
            content_width: popup_area.width.saturating_sub(2),
            is_selected: false,
            is_expanded: false,
            tool_entry_max_lines: DEFAULT_TOOL_ENTRY_MAX_LINES,
            theme: theme.clone(),
            paired_status: None,
            is_streaming: false,
            is_waiting_on_subagent: false,
        };
        render_preview_lines(
            session.history(),
            &ctx,
            PREVIEW_ENTRY_COUNT,
            PREVIEW_MAX_LINES,
        )
    };

    let (mut terminal, _) = setup_term(term_width, term_height);
    terminal
        .draw(|frame| {
            render_session_preview(frame, popup_area, session, &theme, &lines);
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
    session.set_model(ModelSelection::Single("ollama/llama3".to_owned()));

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
// CWD display tests
// ---------------------------------------------------------------------------

#[rstest::rstest]
fn cwd_shows_on_model_line() {
    // Given a session with a cwd and a model.
    let mut session = ChatSessionState::new();
    session.set_cwd(std::path::PathBuf::from("/home/user/jinn"));
    session.set_model(ModelSelection::Single("ollama/llama3".to_owned()));

    // When rendering the preview.
    let (buffer, popup_area) = render_preview(&session, 80, 40);

    // Then the bottom inner row contains both the cwd and model.
    let model_y = popup_area.y + popup_area.height - 2;
    let row = buffer_row(&buffer, model_y, popup_area.x + popup_area.width);
    assert!(
        row.contains("jinn"),
        "model line should contain cwd 'jinn', got: {row}"
    );
    assert!(
        row.contains("(ollama)/llama3"),
        "model line should contain model '(ollama)/llama3', got: {row}"
    );
}

#[rstest::rstest]
fn cwd_left_truncated_when_long() {
    // Given a session with a very long cwd.
    let mut session = ChatSessionState::new();
    let long_cwd = "/very/long/path/that/should/be/truncated/to/fit/the/popup/jinn";
    session.set_cwd(std::path::PathBuf::from(long_cwd));
    session.set_model(ModelSelection::Single("ollama/llama3".to_owned()));

    // When rendering the preview.
    let (buffer, popup_area) = render_preview(&session, 80, 40);

    // Then the bottom inner row shows the cwd left-truncated with '…'.
    let model_y = popup_area.y + popup_area.height - 2;
    let row = buffer_row(&buffer, model_y, popup_area.x + popup_area.width);
    assert!(
        row.contains('\u{2026}'),
        "model line should contain '…' when cwd is truncated, got: {row}"
    );
    assert!(
        row.contains("jinn"),
        "truncated cwd should preserve trailing 'jinn', got: {row}"
    );
}

#[rstest::rstest]
fn cwd_shows_with_no_model_selected() {
    // Given a session with a cwd but no model.
    let mut session = ChatSessionState::new();
    session.set_cwd(std::path::PathBuf::from("/home/user/jinn"));

    // When rendering the preview.
    let (buffer, popup_area) = render_preview(&session, 80, 40);

    // Then the bottom inner row shows both cwd and 'no model selected'.
    let model_y = popup_area.y + popup_area.height - 2;
    let row = buffer_row(&buffer, model_y, popup_area.x + popup_area.width);
    assert!(
        row.contains("jinn"),
        "model line should contain cwd 'jinn', got: {row}"
    );
    assert!(
        row.contains("no model selected"),
        "model line should contain 'no model selected', got: {row}"
    );
}

// ---------------------------------------------------------------------------
// Completion badge tests
// ---------------------------------------------------------------------------

/// Builds a session whose task list has `total` tasks, `completed` of which
/// are marked [`TaskStatus::Completed`].
fn session_with_tasks(completed: usize, total: usize) -> ChatSessionState {
    let mut session = ChatSessionState::new();
    let tasks = (0..completed)
        .map(|_| ("done".to_owned(), TaskStatus::Completed))
        .chain((completed..total).map(|_| ("todo".to_owned(), TaskStatus::Pending)))
        .collect();
    session.task_list_mut().set_from_inputs(&[PhaseInput {
        description: "Build".to_owned(),
        tasks,
    }]);
    session
}

#[rstest::rstest]
fn badge_renders_counts_and_percentage_in_top_border() {
    // Given a session with 3 tasks, 1 completed (1*100/3 = 33%).
    let session = session_with_tasks(1, 3);

    // When rendering the preview.
    let (buffer, popup_area) = render_preview(&session, 80, 40);

    // Then the top border row contains the badge "1/3 · 33%".
    let top_row = buffer_row(&buffer, popup_area.y, popup_area.x + popup_area.width);
    assert!(
        top_row.contains("1/3"),
        "top border should contain '1/3' badge, got: {top_row}"
    );
    assert!(
        top_row.contains("33%"),
        "top border should contain '33%' (truncated), got: {top_row}"
    );
    assert!(
        top_row.contains('\u{00B7}'),
        "top border should contain '·' separator, got: {top_row}"
    );
}

#[rstest::rstest]
fn badge_hidden_when_task_list_empty() {
    // Given a session with no tasks.
    let mut session = ChatSessionState::new();
    session.set_title("No Tasks".to_owned());

    // When rendering the preview.
    let (buffer, popup_area) = render_preview(&session, 80, 40);

    // Then the top border row has no '%' badge but still shows the title.
    let top_row = buffer_row(&buffer, popup_area.y, popup_area.x + popup_area.width);
    assert!(
        !top_row.contains('%'),
        "top border should not contain a percentage badge when empty, got: {top_row}"
    );
    assert!(
        top_row.contains("No Tasks"),
        "top border should still show the session title, got: {top_row}"
    );
}

#[rstest::rstest]
fn badge_uses_streaming_color() {
    // Given a session with tasks.
    let streaming = default_theme().streaming;
    let session = session_with_tasks(1, 3);

    // When rendering the preview.
    let (buffer, popup_area) = render_preview(&session, 80, 40);

    // Then at least one cell in the top border row uses the streaming color.
    let top_border_y = popup_area.y;
    let has_streaming = (popup_area.x..popup_area.x + popup_area.width)
        .filter_map(|x| buffer.cell((x, top_border_y)))
        .any(|cell| cell.fg == streaming);
    assert!(
        has_streaming,
        "some top border cell should use the streaming color {streaming:?}"
    );
}

// ---------------------------------------------------------------------------
// Regression tests: cursor-relative positioning with 2-row gap
// ---------------------------------------------------------------------------

#[rstest::rstest]
#[case::normal(30, 5)]
#[case::content_exceeds_space(30, 20)]
#[case::cursor_near_top(10, 5)]
fn popup_bottom_edge_sits_two_rows_above_cursor(
    #[case] cursor_y: u16,
    #[case] content_line_count: usize,
) {
    // Given a frame area and a cursor position.
    let frame_area = Rect::new(0, 0, 80, 40);

    // When computing the popup rect.
    let popup_rect = session_preview_popup_rect(frame_area, cursor_y, content_line_count);

    // Then the popup bottom edge + 2 = cursor_y, leaving a one-row gap.
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

    // Then each popup is anchored to its cursor (1-row gap invariant).
    assert_eq!(
        rect_at_20.y + rect_at_20.height + 2,
        20,
        "popup at cursor_y=20 should satisfy the gap invariant"
    );
    assert_eq!(
        rect_at_25.y + rect_at_25.height + 2,
        25,
        "popup at cursor_y=25 should satisfy the gap invariant"
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
    let popup_rect = session_preview_popup_rect(frame_area, cursor_y, content_line_count);

    // Then the popup height is capped (max_height = 7 - 0 - 2 = 5).
    assert_eq!(
        popup_rect.height, 5,
        "popup height should be capped when cursor is near top, \
         got: {}",
        popup_rect.height
    );

    // And the popup does not extend past the gap boundary toward the cursor.
    assert!(
        popup_rect.y + popup_rect.height < cursor_y,
        "popup should not encroach on the gap above cursor"
    );
}

/// The popup's content area when the render has not come back.
///
/// A frame that renders nothing while it waits is indistinguishable from a
/// frame that is stuck, so the loading state has to be visible.
mod loading_state {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use super::*;
    use jinn_slices::spinner_glyph;

    /// The popup drawn by the loading renderer, at the geometry production uses.
    ///
    /// Sized from [`LOADING_CONTENT_ROWS`], the same constant the render pass
    /// draws with. A hand-picked count here would let the test pass against
    /// geometry production never uses — which is exactly how the spinner went
    /// missing while this test stayed green.
    fn draw_loading(
        session: &ChatSessionState,
        term_width: u16,
        term_height: u16,
    ) -> ratatui::buffer::Buffer {
        let theme = default_theme();
        let frame_area = Rect::new(0, 0, term_width, term_height);
        let popup_area =
            session_preview_popup_rect(frame_area, 30, LOADING_CONTENT_ROWS);

        let (mut terminal, _) = setup_term(term_width, term_height);
        terminal
            .draw(|frame| {
                render_session_preview_loading(frame, popup_area, session, &theme);
            })
            .expect("draw");
        terminal.backend().buffer().clone()
    }

    /// The buffer as one string, for a whole-popup assertion.
    fn buffer_to_string(buffer: &ratatui::buffer::Buffer) -> String {
        let area = buffer.area();
        (area.y..area.y + area.height)
            .map(|y| buffer_row(buffer, y, area.x + area.width))
            .collect()
    }

    /// Every glyph `spinner_glyph` can return, so the assertion is not tied to
    /// which frame the test happens to catch.
    fn any_spinner_glyph() -> Vec<String> {
        (0..8)
            .map(|step| {
                spinner_glyph(std::time::Duration::from_millis(
                    u64::try_from(step).unwrap_or(0)
                        * u64::try_from(jinn_slices::SPINNER_INTERVAL.as_millis()).unwrap_or(1),
                ))
                .to_owned()
            })
            .collect()
    }

    #[rstest::rstest]
    fn the_loading_popup_reserves_a_content_row() {
        // Given a frame with room above the cursor for a small popup.
        let frame_area = Rect::new(0, 0, 100, 40);

        // When the loading popup's rect is computed the way production does.
        let popup_area =
            session_preview_popup_rect(frame_area, 30, LOADING_CONTENT_ROWS);

        // Then rows remain for content once the borders and footer are taken.
        // At the popup's 5-row floor this would be zero, and the content guard
        // would drop the spinner line entirely.
        let content_rows = popup_area.height.saturating_sub(2).saturating_sub(3);
        assert!(
            content_rows > 0,
            "the loading popup must leave room for the spinner, got {content_rows} content rows in a {} row popup",
            popup_area.height
        );
    }

    #[rstest::rstest]
    fn the_loading_state_shows_a_spinner() {
        // Given a session whose preview has not been rendered.
        let session = make_session_with_title("busy");
        let buffer = draw_loading(&session, 100, 40);

        // Then the content area carries a spinner glyph.
        let screen = buffer_to_string(&buffer);
        assert!(
            any_spinner_glyph().iter().any(|g| screen.contains(g)),
            "expected a spinner glyph on screen"
        );
    }

    #[rstest::rstest]
    fn the_loading_state_does_not_show_entry_text() {
        // Given a session whose entries carry distinctive text.
        let mut session = make_session_with_title("busy");
        session.push_entry(ChatEntry::user("SECRETENTRYTEXT"));
        let buffer = draw_loading(&session, 100, 40);

        // Then none of it is drawn, because the render has not come back.
        let screen = buffer_to_string(&buffer);
        assert!(
            !screen.contains("SECRETENTRYTEXT"),
            "a loading popup must not draw text it has not been given"
        );
    }

    #[rstest::rstest]
    fn the_loading_state_shows_the_session_title() {
        // Given a titled session whose preview has not been rendered.
        let session = make_session_with_title("busy");
        let buffer = draw_loading(&session, 100, 40);

        // Then the chrome is already drawn, so only the content waits.
        assert!(buffer_to_string(&buffer).contains("busy"));
    }

    #[rstest::rstest]
    fn the_loading_state_shows_the_keybinds() {
        // Given a session whose preview has not been rendered.
        let session = make_session_with_title("busy");
        let buffer = draw_loading(&session, 100, 40);

        // Then the footer is present, matching the ready state.
        assert!(buffer_to_string(&buffer).contains("archive"));
    }

    #[rstest::rstest]
    fn a_session_with_no_entries_draws_no_content() {
        // Given a session with an empty history, rendered and complete.
        let session = make_session_with_title("empty");
        let lines: Vec<ratatui::text::Line<'static>> = Vec::new();

        // When the ready renderer draws it.
        let theme = default_theme();
        let frame_area = Rect::new(0, 0, 100, 40);
        let popup_area = session_preview_popup_rect(frame_area, 30, 0);
        let (mut terminal, _) = setup_term(100, 40);
        terminal
            .draw(|frame| {
                render_session_preview(frame, popup_area, &session, &theme, &lines);
            })
            .expect("draw");

        // Then the popup is chrome-only — no spinner, because it is not waiting.
        let screen = buffer_to_string(terminal.backend().buffer());
        assert!(
            !any_spinner_glyph().iter().any(|g| screen.contains(g)),
            "an empty preview is complete, not loading"
        );
    }
}
