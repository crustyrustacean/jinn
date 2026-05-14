//! Assistant entry rendering — cyan, no icon prefix.

use ratatui::style::{Color, Style};
use ratatui::text::Line;

use super::shared::multiline_styled;

pub fn to_lines(text: &str, pinned: bool, is_selected: bool) -> Vec<Line<'static>> {
    let prefix = if pinned { "📌 " } else { "" };
    multiline_styled(
        text,
        prefix,
        "  ",
        Style::default().fg(Color::Cyan),
        is_selected,
    )
}
