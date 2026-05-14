//! System entry rendering — dark gray with indentation.

use ratatui::style::{Color, Style};
use ratatui::text::Line;

use super::shared::{RenderContext, multiline_styled};

pub fn to_lines(text: &str, _ctx: &RenderContext) -> Vec<Line<'static>> {
    multiline_styled(text, "", "", Style::default().fg(Color::DarkGray))
}
