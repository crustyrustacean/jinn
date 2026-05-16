//! Actor entry rendering — yellow with source name.

use ratatui::style::Style;
use ratatui::text::Line;

use super::shared::{RenderContext, multiline_styled, pad_entry, Pad};

pub fn to_lines(source: &str, text: &str, ctx: &RenderContext) -> Vec<Line<'static>> {
    let prefix = format!("[actor] {source}: ");
    let mut lines = multiline_styled(
        text,
        &prefix,
        "",
        Style::default().fg(ctx.theme.focus_accent),
    );
    pad_entry(&mut lines, Pad::Both);
    lines
}
