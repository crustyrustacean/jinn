//! Reasoning effort picker entry type.

use jinn_theme::Theme;

use super::ReasoningEffort;

/// A reasoning effort entry ready for display in the picker.
///
/// Carries the [`ReasoningEffort`] variant so the confirm handler can read it
/// back without re-parsing the display name.
#[derive(Debug, Clone)]
pub struct ReasoningEffortEntry {
    /// The effort variant this entry represents.
    pub effort: ReasoningEffort,
    /// Human-readable display name (the wire string, e.g. "high").
    pub name: String,
    /// Short human description (e.g. "High effort").
    pub description: String,
    /// Whether this is the currently resolved effort.
    pub is_active: bool,
    /// Theme for rendering.
    pub theme: Theme,
}

impl jinn_selection_widget::TreeItem for ReasoningEffortEntry {
    fn id(&self) -> &str {
        &self.name
    }

    fn parent_id(&self) -> Option<&str> {
        None
    }

    fn display_label(&self) -> &str {
        &self.name
    }

    fn render_row(&self, _is_selected: bool) -> ratatui::text::Line<'static> {
        // Rows render through the spec's row hook via PickerEntry; this
        // impl only supplies tree structure (id/parent_id) and filter text.
        ratatui::text::Line::raw(self.display_label().to_owned())
    }

    fn render_row_with_highlight(
        &self,
        _is_selected: bool,
        _match_indices: &[std::ops::Range<usize>],
    ) -> ratatui::text::Line<'static> {
        ratatui::text::Line::raw(self.display_label().to_owned())
    }
}
