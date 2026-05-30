//! Preview content trait — provides renderable lines for a preview pane.
//!
//! Items that want to show a preview in the picker implement this trait.
//! The preview widget calls [`PreviewContent::preview_lines`] to get styled
//! lines for display.

use ratatui::text::Line;

/// Trait for picker items that can provide preview content.
///
/// Implementors return styled lines for display in the preview pane.
/// The `width` parameter allows word-wrapping to the available pane width.
pub trait PreviewContent {
    /// Returns the preview lines for display in the preview pane.
    ///
    /// `width` is the number of columns available for rendering.
    /// Implementors should wrap text to fit within this width.
    fn preview_lines(&self, width: usize) -> Vec<Line<'static>>;
}
