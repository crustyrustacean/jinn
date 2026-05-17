//! Info entry rendering — styled lines with padding.
//!
//! Info entries are UI-only messages (welcome hints, etc.) that are
//! excluded from prompt assembly and LLM context. They carry pre-built
//! styled `Line`s for rich formatting and render with padding.

use ratatui::text::Line;

use super::shared::{Pad, RenderContext, pad_entry};

/// Render pre-built info lines with padding.
///
/// Clones the lines (to satisfy `&mut Vec` padding) and adds blank
/// padding above and below for visual spacing.
pub fn to_lines(lines: &[Line<'static>], _ctx: &RenderContext) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = lines.to_vec();
    pad_entry(&mut lines, Pad::Both);
    lines
}
