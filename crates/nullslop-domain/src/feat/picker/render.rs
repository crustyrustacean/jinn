//! Picker rendering for keymap, persona, and theme pickers.

use crate::common::app_state::AppState;
use nullslop_selection_widget::SelectionWidget;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;

/// Renders the keymap picker overlay using [`SelectionWidget`].
///
/// Telescope-style layout: bordered popup with filter input at top,
/// horizontal separator, scrollable keymap entries, and a footer showing
/// the scope filter mode.
pub fn render_keymap_picker(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let scope_name = state
        .frontend
        .scope_stack
        .parent()
        .map_or_else(|| "unknown".to_owned(), std::string::ToString::to_string);
    let footer = if state.frontend.keymap_picker_show_all {
        Line::from(format!(" All scopes | CTRL+A to show {scope_name} "))
    } else {
        Line::from(format!(" Scope: {scope_name} | CTRL+A to show all "))
    };
    let widget = SelectionWidget::new(&state.frontend.keymap_picker)
        .title(Line::from(" Keymaps "))
        .title_style(Style::default().fg(state.frontend.theme.popup_title))
        .footer(footer);
    widget.render(frame, area);
}

/// Renders the persona picker overlay using [`SelectionWidget`].
///
/// Telescope-style layout: bordered popup with filter input at top,
/// horizontal separator, scrollable persona entries.
pub fn render_persona_picker(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let footer = {
        use ratatui::style::Style;
        use ratatui::text::{Line, Span};
        let gray = Style::default().fg(state.frontend.theme.muted_text);
        let active_name = state
            .context
            .active_persona
            .as_ref()
            .map_or("none", |p| p.name.as_str());
        Line::from(vec![
            Span::styled("Active: ".to_owned(), gray),
            Span::styled(
                active_name.to_owned(),
                Style::default().fg(state.frontend.theme.primary_text),
            ),
        ])
    };
    let widget = SelectionWidget::new(&state.frontend.persona_picker)
        .title(Line::from(" Personas "))
        .title_style(Style::default().fg(state.frontend.theme.popup_title))
        .footer(footer);
    widget.render(frame, area);
}

/// Renders the theme picker overlay using [`SelectionWidget`].
///
/// Telescope-style layout: bordered popup with filter input at top,
/// horizontal separator, scrollable theme entries.
pub fn render_theme_picker(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let widget = SelectionWidget::new(&state.frontend.theme_picker)
        .title(Line::from(" Themes "))
        .title_style(Style::default().fg(state.frontend.theme.popup_title))
        .footer(Line::from(" ESC to cancel, Enter to apply "));
    widget.render(frame, area);
}
