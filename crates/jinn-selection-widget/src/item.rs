//! Picker item trait - the consumer-facing contract.
//!
//! Consumers implement [`PickerItem`] for their domain type (e.g., `ProviderEntry`,
//! `ActorEntry`). The widget uses [`PickerItem::display_label`] for fuzzy matching and
//! [`PickerItem::render_row`] for styled display.

use ratatui::text::Line;

use std::ops::Range;

/// Byte-offset ranges within [`display_label`](PickerItem::display_label) that matched
/// the current fuzzy filter. Used by [`render_row_with_highlight`](PickerItem::render_row_with_highlight)
/// to visually distinguish matched characters.
///
/// The ranges are **sorted** and **non-overlapping**, and all offsets are byte indices
/// into the string returned by `display_label()`.
pub type MatchRanges = Vec<Range<usize>>;

/// An item that can be displayed and selected in a picker.
///
/// The widget uses [`display_label`](PickerItem::display_label) for fuzzy matching and
/// [`render_row`](PickerItem::render_row) for styled display in the picker list.
///
/// # Examples
///
/// See the tests in this crate for full implementation examples.
pub trait PickerItem: std::fmt::Debug + 'static {
    /// Returns searchable text used for fuzzy matching.
    ///
    /// Should contain all text the user might search by (name, model, backend, etc.).
    fn display_label(&self) -> &str;

    /// Renders this item as a styled line for display in the picker.
    ///
    /// `is_selected` indicates whether this row is currently highlighted.
    /// The consumer controls all styling - colors, markers, icons, dimming, etc.
    fn render_row(&self, is_selected: bool) -> Line<'static>;

    /// Renders this item with fuzzy match highlighting.
    ///
    /// `is_selected` indicates whether this row is currently highlighted.
    /// `match_indices` contains sorted, non-overlapping byte ranges within
    /// [`display_label`](Self::display_label) that matched the filter.
    /// When `match_indices` is empty (no active filter), delegates to [`render_row`](Self::render_row).
    ///
    /// The default implementation ignores match indices and delegates to `render_row`.
    /// Override this to provide highlighting.
    fn render_row_with_highlight(
        &self,
        is_selected: bool,
        match_indices: &[Range<usize>],
    ) -> Line<'static> {
        let _ = match_indices;
        self.render_row(is_selected)
    }
}
