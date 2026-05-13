//! Shared style helpers for picker entry rendering.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Renders the active-item marker: `> ` when active, `  ` when not.
pub fn active_marker(is_active: bool) -> Span<'static> {
    Span::styled(
        if is_active { "> " } else { "  " },
        if is_active {
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        },
    )
}

/// Returns the style for selected items (white text on dark gray background).
pub fn selected_style(is_selected: bool) -> Style {
    if is_selected {
        Style::default().fg(Color::White).bg(Color::DarkGray)
    } else {
        Style::default()
    }
}

/// Returns a dimmed style for description text. When selected, uses dark gray on dark gray.
pub fn dim_style(is_selected: bool) -> Style {
    if is_selected {
        Style::default().fg(Color::DarkGray).bg(Color::DarkGray)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

/// Builds a footer line with a dark-gray label and white value.
pub fn labeled_footer(label: &str, value: &str) -> Line<'static> {
    let gray = Style::default().fg(Color::DarkGray);
    Line::from(vec![
        Span::styled(format!("{label}: "), gray),
        Span::styled(value.to_owned(), Style::default().fg(Color::White)),
    ])
}

/// Promotes the first active item to the top of the list when the filter is empty.
///
/// This ensures the currently-active item (e.g., active provider, active strategy)
/// always appears first in the unfiltered picker list.
pub fn promote_active_to_top<T, F>(entries: &mut [T], is_active: F, filter: &str)
where
    F: Fn(&T) -> bool,
{
    if filter.is_empty() {
        if let Some(pos) = entries.iter().position(is_active) {
            if pos > 0 {
                #[expect(
                    clippy::indexing_slicing,
                    reason = "pos comes from iter().position() on the same slice"
                )]
                entries[0..=pos].rotate_right(1);
            }
        }
    }
}
