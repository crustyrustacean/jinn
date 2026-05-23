//! Viewport state for panning and node selection.
//!
//! [`ViewportState`] tracks the scroll offset and selected node. The parent TUI
//! calls methods on it to respond to user input. The widget reads from it during
//! rendering to apply clipping and highlight.

/// Viewport state for the workflow visualization widget.
///
/// Owned by the parent TUI, not by the widget itself. The widget reads from this
/// struct each frame.
#[derive(Debug, Clone)]
pub struct ViewportState {
    /// Horizontal scroll offset (cells).
    pub offset_x: u16,
    /// Vertical scroll offset (cells).
    pub offset_y: u16,
    /// Currently selected node name, if any.
    pub selected: Option<String>,
}

impl ViewportState {
    /// Creates a new viewport state with no offset and no selection.
    #[must_use]
    pub fn new() -> Self {
        Self {
            offset_x: 0,
            offset_y: 0,
            selected: None,
        }
    }

    /// Creates a new viewport state with a pre-selected node.
    #[must_use]
    pub fn with_selected(name: String) -> Self {
        Self {
            offset_x: 0,
            offset_y: 0,
            selected: Some(name),
        }
    }

    /// Returns the currently selected node name, if any.
    #[must_use]
    pub fn selected_node(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// Pans the viewport up by the given number of cells.
    pub fn pan_up(&mut self, amount: u16) {
        self.offset_y = self.offset_y.saturating_sub(amount);
    }

    /// Pans the viewport down by the given number of cells.
    pub fn pan_down(&mut self, amount: u16) {
        self.offset_y = self.offset_y.saturating_add(amount);
    }

    /// Pans the viewport left by the given number of cells.
    pub fn pan_left(&mut self, amount: u16) {
        self.offset_x = self.offset_x.saturating_sub(amount);
    }

    /// Pans the viewport right by the given number of cells.
    pub fn pan_right(&mut self, amount: u16) {
        self.offset_x = self.offset_x.saturating_add(amount);
    }

    /// Selects the next node in the list, cycling back to the first.
    ///
    /// If nothing is selected, selects the first node. If the list is empty,
    /// does nothing.
    #[expect(clippy::indexing_slicing, reason = "next is bounded by node_names.len() via modular arithmetic")]
    pub fn select_next(&mut self, node_names: &[String]) {
        if node_names.is_empty() {
            return;
        }
        let next = match &self.selected {
            None => 0,
            Some(current) => {
                let idx = node_names
                    .iter()
                    .position(|n| n == current)
                    .unwrap_or(0);
                (idx + 1) % node_names.len()
            }
        };
        self.selected = Some(node_names[next].clone());
    }

    /// Selects the previous node in the list, cycling to the last.
    ///
    /// If nothing is selected, selects the last node. If the list is empty,
    /// does nothing.
    #[expect(clippy::indexing_slicing, reason = "prev is bounded by node_names.len() via modular arithmetic")]
    pub fn select_prev(&mut self, node_names: &[String]) {
        if node_names.is_empty() {
            return;
        }
        let prev = match &self.selected {
            None => node_names.len() - 1,
            Some(current) => {
                let idx = node_names
                    .iter()
                    .position(|n| n == current)
                    .unwrap_or(0);
                if idx == 0 {
                    node_names.len() - 1
                } else {
                    idx - 1
                }
            }
        };
        self.selected = Some(node_names[prev].clone());
    }

    /// Checks whether the given node name is the currently selected node.
    #[must_use]
    pub fn is_selected(&self, name: &str) -> bool {
        self.selected.as_deref() == Some(name)
    }
}

impl Default for ViewportState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pan_up_decreases_y() {
        let mut vp = ViewportState::new();
        vp.offset_y = 10;
        vp.pan_up(3);
        assert_eq!(vp.offset_y, 7);
    }

    #[test]
    fn pan_up_clamps_to_zero() {
        let mut vp = ViewportState::new();
        vp.offset_y = 2;
        vp.pan_up(5);
        assert_eq!(vp.offset_y, 0);
    }

    #[test]
    fn pan_down_increases_y() {
        let mut vp = ViewportState::new();
        vp.pan_down(5);
        assert_eq!(vp.offset_y, 5);
    }

    #[test]
    fn pan_left_decreases_x() {
        let mut vp = ViewportState::new();
        vp.offset_x = 10;
        vp.pan_left(3);
        assert_eq!(vp.offset_x, 7);
    }

    #[test]
    fn pan_right_increases_x() {
        let mut vp = ViewportState::new();
        vp.pan_right(5);
        assert_eq!(vp.offset_x, 5);
    }

    #[test]
    fn select_next_cycles_forward() {
        let names: Vec<String> = vec!["a".into(), "b".into(), "c".into()];
        let mut vp = ViewportState::new();

        vp.select_next(&names);
        assert_eq!(vp.selected_node(), Some("a"));

        vp.select_next(&names);
        assert_eq!(vp.selected_node(), Some("b"));

        vp.select_next(&names);
        assert_eq!(vp.selected_node(), Some("c"));

        // Wraps around.
        vp.select_next(&names);
        assert_eq!(vp.selected_node(), Some("a"));
    }

    #[test]
    fn select_prev_cycles_backward() {
        let names: Vec<String> = vec!["a".into(), "b".into(), "c".into()];
        let mut vp = ViewportState::new();

        vp.select_prev(&names);
        assert_eq!(vp.selected_node(), Some("c"));

        vp.select_prev(&names);
        assert_eq!(vp.selected_node(), Some("b"));

        vp.select_prev(&names);
        assert_eq!(vp.selected_node(), Some("a"));

        // Wraps around.
        vp.select_prev(&names);
        assert_eq!(vp.selected_node(), Some("c"));
    }

    #[test]
    fn select_next_empty_does_nothing() {
        let names: Vec<String> = vec![];
        let mut vp = ViewportState::new();
        vp.select_next(&names);
        assert_eq!(vp.selected_node(), None);
    }

    #[test]
    fn is_selected_checks_current() {
        let mut vp = ViewportState::with_selected("b".into());
        assert!(vp.is_selected("b"));
        assert!(!vp.is_selected("a"));
    }
}
