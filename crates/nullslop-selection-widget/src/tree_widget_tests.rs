//! Tests for tree picker widget rendering.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;

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

fn render_to_buffer(state: &TreePickerState<TestItem>) -> Buffer {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).expect("create terminal");
    terminal
        .draw(|frame| {
            let widget = TreePickerWidget::new(state).title(Line::from(" Test Tree "));
            widget.render(frame, frame.area());
        })
        .expect("draw");
    terminal.backend().clone().buffer().clone()
}

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

fn buffer_to_string(buffer: &Buffer) -> String {
    let mut s = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let cell = buffer.cell((x, y)).expect("cell");
            s.push_str(cell.symbol());
        }
        s.push('\n');
    }
    s
}
