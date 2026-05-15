//! Shared style helpers for picker entry rendering.

use crate::feat::theme::Theme;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

/// Renders the active-item marker: `> ` when active, `  ` when not.
pub fn active_marker(is_active: bool, theme: &Theme) -> Span<'static> {
    Span::styled(
        if is_active { "> " } else { "  " },
        if is_active {
            Style::default()
                .fg(theme.picker_active_marker)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        },
    )
}

/// Returns the style for selected items (primary text on selected background).
pub fn selected_style(is_selected: bool, theme: &Theme) -> Style {
    if is_selected {
        Style::default()
            .fg(theme.primary_text)
            .bg(theme.picker_selected_bg)
    } else {
        Style::default()
    }
}

/// Returns a dimmed style for description text. When selected, uses muted text
/// on the selected background.
pub fn dim_style(is_selected: bool, theme: &Theme) -> Style {
    if is_selected {
        Style::default()
            .fg(theme.muted_text)
            .bg(theme.picker_selected_bg)
    } else {
        Style::default().fg(theme.muted_text)
    }
}

/// Builds a footer line with a muted label and primary text value.
pub fn labeled_footer(label: &str, value: &str, theme: &Theme) -> Line<'static> {
    let gray = Style::default().fg(theme.muted_text);
    Line::from(vec![
        Span::styled(format!("{label}: "), gray),
        Span::styled(value.to_owned(), Style::default().fg(theme.primary_text)),
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
    if filter.is_empty()
        && let Some(pos) = entries.iter().position(is_active)
        && pos > 0
    {
        #[expect(
            clippy::indexing_slicing,
            reason = "pos comes from iter().position() on the same slice"
        )]
        entries[0..=pos].rotate_right(1);
    }
}
