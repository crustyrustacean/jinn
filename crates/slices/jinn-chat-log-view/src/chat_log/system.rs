//! System entry rendering - dark gray with indentation.

use ratatui::style::Style;
use ratatui::text::Line;

use super::shared::{Pad, RenderContext, multiline_styled, pad_entry};

pub fn to_lines(text: &str, ctx: &RenderContext) -> Vec<Line<'static>> {
    let text = super::shared::strip_ansi(text);
    let mut lines = multiline_styled(&text, "", "", Style::default().fg(ctx.theme.muted_text));
    pad_entry(&mut lines, Pad::Both);
    lines
}
