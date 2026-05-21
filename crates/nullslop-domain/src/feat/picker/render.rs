//! Picker rendering for persona and theme pickers.

use crate::common::app_state::AppState;
use nullslop_selection_widget::SelectionWidget;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::style::Style;

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
