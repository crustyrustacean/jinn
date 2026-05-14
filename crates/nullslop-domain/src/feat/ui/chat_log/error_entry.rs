//! Error entry rendering — red text.

use ratatui::style::{Color, Style};
use ratatui::text::Line;

use super::shared::{RenderContext, multiline_styled};

pub fn to_lines(text: &str, _ctx: &RenderContext) -> Vec<Line<'static>> {
    multiline_styled(text, "", "", Style::default().fg(Color::Red))
}
