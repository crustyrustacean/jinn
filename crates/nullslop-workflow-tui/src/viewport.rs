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
    /// Horizontal camera offset (cells). Positive = camera right, content shifts left.
    pub offset_x: i32,
    /// Vertical camera offset (cells). Positive = camera down, content shifts up.
    pub offset_y: i32,
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

    /// Translates the camera by (`dx`, `dy`) cells, clamped so content can
    /// scroll at most half its own size off any edge.
    ///
    /// Positive `dx` moves the camera right (content shifts left).
    /// Positive `dy` moves the camera down (content shifts up).
    ///
    /// The offset is clamped to `[-(content / 2), content / 2]`
    /// on each axis. For a 100-wide graph, the offset range is `[-50, 50]`.
    pub fn translate(
        &mut self,
        dx: i32,
        dy: i32,
        content_size: (u16, u16),
    ) {
        let (cw, ch) = content_size;
        let half_cw = i32::from(cw / 2);
        let half_ch = i32::from(ch / 2);

        self.offset_x = self.offset_x.saturating_add(dx);
        self.offset_y = self.offset_y.saturating_add(dy);

        let min_x = -half_cw;
        let max_x = half_cw;
        let min_y = -half_ch;
        let max_y = half_ch;

        self.offset_x = self.offset_x.clamp(min_x, max_x);
        self.offset_y = self.offset_y.clamp(min_y, max_y);
    }

    /// Selects the next node in the list, cycling back to the first.
    ///
    /// If nothing is selected, selects the first node. If the list is empty,
    /// does nothing.
    #[expect(
        clippy::indexing_slicing,
        reason = "next is bounded by node_names.len() via modular arithmetic"
    )]
    pub fn select_next(&mut self, node_names: &[String]) {
        if node_names.is_empty() {
            return;
        }
        let next = match &self.selected {
            None => 0,
            Some(current) => {
                let idx = node_names.iter().position(|n| n == current).unwrap_or(0);
                (idx + 1) % node_names.len()
            }
        };
        self.selected = Some(node_names[next].clone());
    }

    /// Selects the previous node in the list, cycling to the last.
    ///
    /// If nothing is selected, selects the last node. If the list is empty,
    /// does nothing.
    #[expect(
        clippy::indexing_slicing,
        reason = "prev is bounded by node_names.len() via modular arithmetic"
    )]
    pub fn select_prev(&mut self, node_names: &[String]) {
        if node_names.is_empty() {
            return;
        }
        let prev = match &self.selected {
            None => node_names.len() - 1,
            Some(current) => {
                let idx = node_names.iter().position(|n| n == current).unwrap_or(0);
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
    fn translate_positive_dx_moves_camera_right() {
        let mut vp = ViewportState::new();
        vp.translate(5, 0, (200, 200));
        assert_eq!(vp.offset_x, 5);
    }

    #[test]
    fn translate_negative_dx_moves_camera_left() {
        let mut vp = ViewportState::new();
        vp.offset_x = 10;
        vp.translate(-3, 0, (200, 200));
        assert_eq!(vp.offset_x, 7);
    }

    #[test]
    fn translate_positive_dy_moves_camera_down() {
        let mut vp = ViewportState::new();
        vp.translate(0, 5, (200, 200));
        assert_eq!(vp.offset_y, 5);
    }

    #[test]
    fn translate_can_go_negative() {
        let mut vp = ViewportState::new();
        vp.translate(-3, -7, (200, 200));
        assert_eq!(vp.offset_x, -3);
        assert_eq!(vp.offset_y, -7);
    }

    #[test]
    fn translate_clamps_at_negative_half_content() {
        let mut vp = ViewportState::new();
        // Content 200 wide → clamp to -(200/2) = -100.
        vp.translate(-200, 0, (200, 200));
        assert_eq!(vp.offset_x, -100);
    }

    #[test]
    fn translate_clamps_at_half_content() {
        let mut vp = ViewportState::new();
        // Content 200 wide → clamp to 200/2 = 100.
        vp.translate(1000, 0, (200, 200));
        assert_eq!(vp.offset_x, 100);
    }

    #[test]
    fn translate_small_content_has_tight_range() {
        let mut vp = ViewportState::new();
        // Content 40 → clamp range [-20, 20].
        vp.translate(100, 0, (40, 40));
        assert_eq!(vp.offset_x, 20, "should clamp to max (20)");
        vp.translate(-200, 0, (40, 40));
        assert_eq!(vp.offset_x, -20, "should clamp to min (-20)");
    }

    #[test]
    fn translate_zero_content_stays_at_zero() {
        let mut vp = ViewportState::new();
        // Content (0, 0) → clamp range [0, 0].
        vp.translate(10, 10, (0, 0));
        assert_eq!(vp.offset_x, 0);
        assert_eq!(vp.offset_y, 0);
    }

    #[test]
    fn translate_initial_offset_stays_valid() {
        // Given offset (0, 0) is always within [-content/2, content/2].
        let mut vp = ViewportState::new();
        vp.translate(0, 0, (100, 10));
        assert_eq!(vp.offset_x, 0);
        assert_eq!(vp.offset_y, 0);
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
        let vp = ViewportState::with_selected("b".into());
        assert!(vp.is_selected("b"));
        assert!(!vp.is_selected("a"));
    }
}
