//! User entry rendering — bold with `>` prefix.

use ratatui::style::{Modifier, Style};
use ratatui::text::Line;

use super::shared::multiline_styled;

pub fn to_lines(text: &str, pinned: bool, is_selected: bool) -> Vec<Line<'static>> {
    let prefix = if pinned { "📌 > " } else { "> " };
    multiline_styled(
        text,
        prefix,
        "  ",
        Style::default().add_modifier(Modifier::BOLD),
        is_selected,
    )
}
