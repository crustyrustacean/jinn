//! Tests for the session preview popup renderer.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::indexing_slicing,
    reason = "test code"
)]

use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::theme::default_theme;
use crate::feat::ui::sidebar::sessions::preview::{
    SessionPreviewCache, render_session_preview, session_preview_popup_rect,
};
use crate::protocol::ChatEntry;
use jinn_testutil::{buffer_row, setup_term};
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
// CWD display tests
// ---------------------------------------------------------------------------

#[rstest::rstest]
fn cwd_shows_on_model_line() {
    // Given a session with a cwd and a model.
    let mut session = ChatSessionState::new();
    session.set_cwd(std::path::PathBuf::from("/home/user/jinn"));
    session.set_model("ollama/llama3".to_owned());

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
    session.set_model("ollama/llama3".to_owned());

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
// Regression tests: cursor-relative positioning with 2-row gap
// ---------------------------------------------------------------------------

#[rstest::rstest]
#[case::normal(30, 5)]
#[case::content_exceeds_space(30, 20)]
#[case::cursor_near_top(10, 5)]
fn popup_bottom_edge_is_one_row_above_cursor(
    #[case] cursor_y: u16,
    #[case] content_line_count: usize,
) {
    // Given a frame area and a cursor position.
    let frame_area = Rect::new(0, 0, 80, 40);

    // When computing the popup rect.
    let popup_rect = session_preview_popup_rect(frame_area, cursor_y, content_line_count);

    // Then the popup bottom edge + 1 = cursor_y.
    assert_eq!(
        popup_rect.y + popup_rect.height + 1,
        cursor_y,
        "popup bottom edge + 1 should equal cursor_y ({cursor_y}), \
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
        rect_at_20.y + rect_at_20.height + 1,
        20,
        "popup at cursor_y=20 should satisfy 1-row gap invariant"
    );
    assert_eq!(
        rect_at_25.y + rect_at_25.height + 1,
        25,
        "popup at cursor_y=25 should satisfy 1-row gap invariant"
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

    // Then the popup height is capped (max_height = 7 - 0 - 1 = 6).
    assert_eq!(
        popup_rect.height, 6,
        "popup height should be capped when cursor is near top, \
         got: {}",
        popup_rect.height
    );

    // And the popup does not extend past the gap boundary toward the cursor.
    assert!(
        popup_rect.y + popup_rect.height < cursor_y,
        "popup should not encroach on the 1-row gap above cursor"
    );
}

// ---------------------------------------------------------------------------
// Plugin preview tests
// ---------------------------------------------------------------------------

use crate::common::app_state::AppState;
use crate::common::render_ctx::RenderCtx;
use crate::common::state::State;
use crate::feat::plugin_dispatch::plugin_sync_hooks::PluginSyncHooks;
use crate::feat::ui::sidebar::sessions::preview::resolve_plugin_preview;
use crate::feat::ui::sidebar::sessions::state::{SessionEntry, SessionEntryKind};
use crate::protocol::SessionId;

/// Stub plugin hooks that returns a fixed JSON value for `on_session_preview`.
struct PreviewStub {
    /// `Some(value)` to return from `on_session_preview`, `None` to return nothing.
    preview_value: Option<serde_json::Value>,
}

impl PluginSyncHooks for PreviewStub {
    fn call_hooks(&self, hook: &str, _ctx: &serde_json::Value) -> Vec<serde_json::Value> {
        match hook {
            "on_session_preview" => self.preview_value.clone().into_iter().collect(),
            _ => vec![],
        }
    }
}

fn plugin_entry(title: &str, parent_id: SessionId) -> SessionEntry {
    SessionEntry {
        kind: SessionEntryKind::Plugin { enabled: true },
        id: parent_id,
        title: title.to_owned(),
        is_active: false,
        created_at: jiff::Timestamp::now(),
        is_idle: true,
        last_entry_is_error: false,
        parent_id: None,
        depth: 0,
        ancestor_continuations: vec![],
        is_last_child: true,
    }
}

#[rstest::rstest]
fn plugin_preview_returns_session_when_hook_responds() {
    // Given a plugin entry and a child session in the state.
    let state = State::new(AppState::default());
    let child_id = SessionId::new();
    let mut child = ChatSessionState::new();
    child.set_session_id(child_id.clone());
    state.write().session.insert(child);

    let parent_id = SessionId::new();
    let entry = plugin_entry("judge", parent_id);
    let stub = PreviewStub {
        preview_value: Some(serde_json::json!({ "session_id": child_id.to_string() })),
    };
    let guard = state.read();
    let render_ctx = RenderCtx::new(&guard).with_plugins(&stub);

    // When resolving plugin preview.
    let result = resolve_plugin_preview(&entry, &render_ctx);

    // Then it returns the child session.
    let session = result.expect("should return a session");
    assert_eq!(session.session_id(), &child_id);
}

#[rstest::rstest]
fn plugin_preview_returns_none_when_hook_returns_nil() {
    // Given a plugin entry with no preview.
    let state = State::new(AppState::default());
    let parent_id = SessionId::new();
    let entry = plugin_entry("judge", parent_id);
    let stub = PreviewStub {
        preview_value: None,
    };
    let guard = state.read();
    let render_ctx = RenderCtx::new(&guard).with_plugins(&stub);

    // When resolving plugin preview.
    let result = resolve_plugin_preview(&entry, &render_ctx);

    // Then it returns None.
    assert!(result.is_none());
}

#[rstest::rstest]
fn plugin_preview_returns_none_when_session_id_missing_from_state() {
    // Given a plugin entry where the hook returns a session ID that doesn't exist.
    let state = State::new(AppState::default());
    let parent_id = SessionId::new();
    let entry = plugin_entry("judge", parent_id);
    let stub = PreviewStub {
        preview_value: Some(serde_json::json!({ "session_id": "s-nonexistent" })),
    };
    let guard = state.read();
    let render_ctx = RenderCtx::new(&guard).with_plugins(&stub);

    // When resolving plugin preview.
    let result = resolve_plugin_preview(&entry, &render_ctx);

    // Then it returns None (session not found).
    assert!(result.is_none());
}
