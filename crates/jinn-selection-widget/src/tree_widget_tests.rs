#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test file, panics are acceptable"
)]

//! Tests for tree picker widget rendering.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use crate::TreePickerState;
use crate::TreePickerWidget;
use crate::tree_item::TreeItem;
use std::ops::Range;

// ---------------------------------------------------------------------------
// Test item type (reuses pattern from tree_state_tests)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct TestItem {
    id: String,
    parent_id: Option<String>,
    label: String,
}

impl TreeItem for TestItem {
    fn id(&self) -> &str {
        &self.id
    }
    fn parent_id(&self) -> Option<&str> {
        self.parent_id.as_deref()
    }
    fn display_label(&self) -> &str {
        &self.label
    }
    fn render_row(&self, _is_selected: bool) -> Line<'static> {
        Line::from(self.label.clone())
    }
    fn render_row_with_highlight(
        &self,
        is_selected: bool,
        _match_indices: &[Range<usize>],
    ) -> Line<'static> {
        self.render_row(is_selected)
    }
}

fn item(id: &str, parent_id: Option<&str>, label: &str) -> TestItem {
    TestItem {
        id: id.to_owned(),
        parent_id: parent_id.map(str::to_owned),
        label: label.to_owned(),
    }
}

/// Test item whose `render_row_with_tree` override embeds the connector handed
/// by the widget between angle brackets, proving placement is the item's call.
#[derive(Debug, Clone)]
struct PrefixEmbeddingItem {
    id: String,
    parent_id: Option<String>,
    label: String,
}

impl TreeItem for PrefixEmbeddingItem {
    fn id(&self) -> &str {
        &self.id
    }
    fn parent_id(&self) -> Option<&str> {
        self.parent_id.as_deref()
    }
    fn display_label(&self) -> &str {
        &self.label
    }
    fn render_row(&self, _is_selected: bool) -> Line<'static> {
        Line::from(self.label.clone())
    }
    fn render_row_with_highlight(
        &self,
        is_selected: bool,
        _match_indices: &[Range<usize>],
    ) -> Line<'static> {
        self.render_row(is_selected)
    }
    fn render_row_with_tree(
        &self,
        is_selected: bool,
        match_ranges: &[Range<usize>],
        tree_prefix: &str,
        tree_style: Style,
    ) -> Line<'static> {
        let mut line = self.render_row_with_highlight(is_selected, match_ranges);
        let mut spans = Vec::with_capacity(line.spans.len() + 2);
        spans.push(Span::styled("<", tree_style));
        spans.push(Span::styled(tree_prefix.to_owned(), tree_style));
        spans.push(Span::styled(">", tree_style));
        spans.extend(line.spans);
        line.spans = spans;
        line
    }
}

fn embedding_item(id: &str, parent_id: Option<&str>, label: &str) -> PrefixEmbeddingItem {
    PrefixEmbeddingItem {
        id: id.to_owned(),
        parent_id: parent_id.map(str::to_owned),
        label: label.to_owned(),
    }
}

fn render_to_buffer<I>(state: &TreePickerState<I>) -> Buffer
where
    I: TreeItem,
{
    render_with(state, |widget| widget)
}

fn render_with<I>(
    state: &TreePickerState<I>,
    configure: impl FnOnce(TreePickerWidget<'_, I>) -> TreePickerWidget<'_, I>,
) -> Buffer
where
    I: TreeItem,
{
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).expect("create terminal");
    terminal
        .draw(|frame| {
            let widget = configure(TreePickerWidget::new(state).title(Line::from(" Test Tree ")));
            widget.render(frame, frame.area());
        })
        .expect("draw");
    terminal.backend().clone().buffer().clone()
}

#[rstest::rstest]
#[test]
fn tree_prefix_rendered_for_child_entry() {
    // Given: root A → child B.
    let items = vec![item("a", None, "Alpha"), item("b", Some("a"), "Bravo")];
    let state = TreePickerState::with_items(items);

    // When rendering.
    let buffer = render_to_buffer(&state);

    // Then the buffer contains tree prefix characters (└─).
    let content = buffer_to_string(&buffer);
    assert!(
        content.contains("└─") || content.contains("├─"),
        "expected tree prefix characters in output"
    );
}

#[rstest::rstest]
#[test]
fn no_tree_prefix_for_root_entry() {
    // Given: only root items (no children).
    let items = vec![item("a", None, "Alpha"), item("b", None, "Bravo")];
    let state = TreePickerState::with_items(items);

    // When rendering.
    let buffer = render_to_buffer(&state);

    // Then no tree prefix characters in output (excluding border chars).
    let content = buffer_to_string(&buffer);
    // Tree prefixes are 3-char sequences. The border uses single │ chars.
    // Check specifically for the 3-char patterns.
    assert!(
        !content.contains("└─ ") && !content.contains("├─ "),
        "expected no tree prefix characters for root-only entries"
    );
}

#[rstest::rstest]
#[test]
fn widget_renders_title() {
    // Given: state with items and a title.
    let items = vec![item("a", None, "Alpha")];
    let state = TreePickerState::with_items(items);

    // When rendering with title " Test Tree ".
    let buffer = render_to_buffer(&state);

    // Then the title appears in the buffer.
    let content = buffer_to_string(&buffer);
    assert!(content.contains("Test Tree"), "expected title in output");
}

#[rstest::rstest]
#[test]
fn widget_passes_child_entries_the_computed_connector() {
    // Given a tree (root Alpha → single child Bravo) whose items embed the
    // handed prefix.
    let items = vec![
        embedding_item("a", None, "Alpha"),
        embedding_item("b", Some("a"), "Bravo"),
    ];
    let state = TreePickerState::with_items(items);

    // When rendering.
    let buffer = render_to_buffer(&state);

    // Then the child row shows exactly the own-level connector, with no
    // root-level continuation segment before it.
    let content = buffer_to_string(&buffer);
    assert!(
        content.contains("<└─ >Bravo"),
        "expected child row to embed exactly `└─ `, got:\n{content}"
    );
}

#[rstest::rstest]
#[test]
fn widget_passes_children_with_siblings_a_tee_connector() {
    // Given a tree with root Alpha and children Bravo, Charlie.
    let items = vec![
        embedding_item("a", None, "Alpha"),
        embedding_item("b", Some("a"), "Bravo"),
        embedding_item("c", Some("a"), "Charlie"),
    ];
    let state = TreePickerState::with_items(items);

    // When rendering.
    let buffer = render_to_buffer(&state);

    // Then the non-last child gets `├─ ` and the last child gets `└─ `.
    let content = buffer_to_string(&buffer);
    assert!(
        content.contains("<├─ >Bravo"),
        "expected non-last child to embed `├─ `, got:\n{content}"
    );
    assert!(
        content.contains("<└─ >Charlie"),
        "expected last child to embed `└─ `, got:\n{content}"
    );
}

#[rstest::rstest]
#[test]
fn widget_passes_grandchildren_a_blank_root_segment() {
    // Given a tree three levels deep: Alpha → Bravo → Charlie.
    let items = vec![
        embedding_item("a", None, "Alpha"),
        embedding_item("b", Some("a"), "Bravo"),
        embedding_item("c", Some("b"), "Charlie"),
    ];
    let state = TreePickerState::with_items(items);

    // When rendering.
    let buffer = render_to_buffer(&state);

    // Then the grandchild embeds one blank segment for the root level plus
    // its own connector (no `│` through the root column).
    let content = buffer_to_string(&buffer);
    assert!(
        content.contains("<   └─ >Charlie"),
        "expected grandchild to embed `   └─ `, got:\n{content}"
    );
}

#[rstest::rstest]
#[test]
fn widget_passes_root_entries_an_empty_connector() {
    // Given a single root item that embeds the handed prefix.
    let state = TreePickerState::with_items(vec![embedding_item("a", None, "Alpha")]);

    // When rendering.
    let buffer = render_to_buffer(&state);

    // Then the root row shows empty brackets (no connector glyphs handed).
    let content = buffer_to_string(&buffer);
    assert!(
        content.contains("<>Alpha"),
        "expected root row to receive an empty connector, got:\n{content}"
    );
    // And no tree connector glyphs were handed anywhere.
    assert!(
        !content.contains("<└─") && !content.contains("<├─"),
        "expected no connector glyphs for root-only entries"
    );
}

#[rstest::rstest]
#[test]
fn widget_passes_configured_prefix_style_to_item_overrides() {
    // Given a tree whose items embed the handed prefix.
    let items = vec![
        embedding_item("a", None, "Alpha"),
        embedding_item("b", Some("a"), "Bravo"),
    ];
    let state = TreePickerState::with_items(items);

    // When rendering with the connector color set to Red.
    let buffer = render_with(&state, |widget| widget.tree_prefix_color(Color::Red));

    // Then the embedded connector brackets carry the configured color.
    let (x, y) = find_cell(&buffer, "<").expect("embedded bracket on screen");
    #[expect(clippy::expect_used, reason = "buffer cell always exists within area")]
    let cell = buffer.cell((x, y)).expect("cell");
    assert_eq!(cell.style().fg, Some(Color::Red));
}

#[rstest::rstest]
#[test]
fn default_render_row_with_tree_prepends_styled_connector() {
    // Given a plain item using the default trait implementation.
    let test_item = item("a", None, "Alpha");
    let style = Style::default().fg(Color::Red);

    // When rendering with a non-empty tree prefix.
    let line = test_item.render_row_with_tree(false, &[], "└─ ", style);

    // Then the first span is the styled connector.
    let connector = line.spans.first().expect("connector span");
    assert_eq!(connector.content, "└─ ");
    assert_eq!(connector.style, style);
    // And the remaining span is the item content.
    let content = line.spans.get(1).expect("content span");
    assert_eq!(content.content, "Alpha");
}

#[rstest::rstest]
#[test]
fn default_render_row_with_tree_returns_content_unwrapped_when_prefix_empty() {
    // Given a plain item using the default trait implementation.
    let test_item = item("a", None, "Alpha");

    // When rendering with an empty tree prefix.
    let line = test_item.render_row_with_tree(false, &[], "", Style::default());

    // Then the content line is returned as-is.
    assert_eq!(line.spans.len(), 1);
    let content = line.spans.first().expect("content span");
    assert_eq!(content.content, "Alpha");
}

fn find_cell(buffer: &Buffer, symbol: &str) -> Option<(u16, u16)> {
    (0..buffer.area.height).find_map(|y| {
        (0..buffer.area.width).find_map(|x| {
            let cell = buffer.cell((x, y))?;
            (cell.symbol() == symbol).then_some((x, y))
        })
    })
}

fn buffer_to_string(buffer: &Buffer) -> String {
    let mut s = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            #[expect(clippy::expect_used, reason = "buffer cell always exists within area")]
            let cell = buffer.cell((x, y)).expect("cell");
            s.push_str(cell.symbol());
        }
        s.push('\n');
    }
    s
}
