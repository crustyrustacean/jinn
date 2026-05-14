//! Keymap and context strategy picker rendering.

use crate::common::app_state::AppState;
use nullslop_selection_widget::SelectionWidget;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;

use super::strategy_entries;

/// Renders the context strategy picker overlay using [`SelectionWidget`].
///
/// Telescope-style layout: bordered popup with filter input at top,
/// horizontal separator, scrollable strategy entries, and a footer showing
/// the current strategy.
pub fn render_context_strategy_picker(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    // Find the active strategy's display name for the footer.
    let active_name = state
        .frontend
        .context_strategy_picker
        .items()
        .iter()
        .find(|e| e.is_active)
        .map_or("unknown", |e| e.name.as_str());

    let footer = strategy_entries::format_strategy_footer(active_name);
    let widget = SelectionWidget::new(&state.frontend.context_strategy_picker)
        .title(Line::from(" Context Assembly Strategy "))
        .footer(footer);
    widget.render(frame, area);
}

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
        .footer(footer);
    widget.render(frame, area);
}

/// Renders the persona picker overlay using [`SelectionWidget`].
///
/// Telescope-style layout: bordered popup with filter input at top,
/// horizontal separator, scrollable persona entries.
pub fn render_persona_picker(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let footer = {
        use ratatui::style::{Color, Style};
        use ratatui::text::{Line, Span};
        let gray = Style::default().fg(Color::DarkGray);
        let active_name = state
            .context
            .active_persona
            .as_ref()
            .map_or("none", |p| p.name.as_str());
        Line::from(vec![
            Span::styled("Active: ".to_owned(), gray),
            Span::styled(active_name.to_owned(), Style::default().fg(Color::White)),
        ])
    };
    let widget = SelectionWidget::new(&state.frontend.persona_picker)
        .title(Line::from(" Personas "))
        .footer(footer);
    widget.render(frame, area);
}
