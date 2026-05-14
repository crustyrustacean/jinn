//! Assistant entry rendering — white text, no background.

use ratatui::style::{Color, Style};
use ratatui::text::Line;

use super::shared::{multiline_styled, RenderContext};

pub fn to_lines(text: &str, _ctx: &RenderContext) -> Vec<Line<'static>> {
    let style = Style::default().fg(Color::White);
    multiline_styled(text, "", "", style)
}
