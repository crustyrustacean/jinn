//! Actor entry rendering — yellow with source name.

use ratatui::style::{Color, Style};
use ratatui::text::Line;

use super::shared::multiline_styled;

pub fn to_lines(source: &str, text: &str, pinned: bool, is_selected: bool) -> Vec<Line<'static>> {
    let base = format!("[actor] {source}: ");
    let prefix = if pinned { format!("📌 {base}") } else { base };
    multiline_styled(
        text,
        &prefix,
        "  ",
        Style::default().fg(Color::Yellow),
        is_selected,
    )
}
