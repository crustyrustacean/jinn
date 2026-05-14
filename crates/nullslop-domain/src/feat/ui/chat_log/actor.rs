//! Actor entry rendering — yellow with source name.

use ratatui::style::{Color, Style};
use ratatui::text::Line;

use super::shared::{multiline_styled, RenderContext};

pub fn to_lines(source: &str, text: &str, _ctx: &RenderContext) -> Vec<Line<'static>> {
    let prefix = format!("[actor] {source}: ");
    multiline_styled(
        text,
        &prefix,
        "",
        Style::default().fg(Color::Yellow),
    )
}
