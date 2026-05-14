//! Tool call entry rendering — magenta with wrench icon.

use ratatui::style::{Color, Style};
use ratatui::text::Line;

use super::shared::multiline_styled;

pub fn to_lines(
    name: &str,
    arguments: &str,
    pinned: bool,
    is_selected: bool,
) -> Vec<Line<'static>> {
    let prefix = if pinned { "📌 " } else { "  " };
    multiline_styled(
        format!("🔧 {name}({arguments})"),
        prefix,
        "  ",
        Style::default().fg(Color::Magenta),
        is_selected,
    )
}
