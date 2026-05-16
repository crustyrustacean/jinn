//! Info entry rendering — dark gray with indentation.
//!
//! Info entries are UI-only messages (welcome hints, etc.) that are
//! excluded from prompt assembly and LLM context. They render identically
//! to system entries: dark gray text with padding.

use ratatui::style::Style;
use ratatui::text::Line;

use super::shared::{Pad, RenderContext, multiline_styled, pad_entry};

pub fn to_lines(text: &str, ctx: &RenderContext) -> Vec<Line<'static>> {
    let mut lines = multiline_styled(text, "", "", Style::default().fg(ctx.theme.muted_text));
    pad_entry(&mut lines, Pad::Both);
    lines
}
