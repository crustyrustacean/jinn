//! User entry rendering — white on light gray background block.

use ratatui::style::{Color, Style};
use ratatui::text::Line;

use super::shared::{multiline_styled, pad_line_to_width, RenderContext};

/// Background color for user entry BLOCK.
const USER_BG: Color = Color::Rgb(0x34, 0x35, 0x41);

pub fn to_lines(text: &str, ctx: &RenderContext) -> Vec<Line<'static>> {
    let style = Style::default().fg(Color::White).bg(USER_BG);
    let mut lines = multiline_styled(text, "", "", style);
    // Pad each line to full content width for BLOCK effect.
    for line in &mut lines {
        pad_line_to_width(line, ctx.content_width, Style::default().bg(USER_BG));
    }
    lines
}
