//! Keymap and context strategy picker rendering.

use nullslop_component::AppState;
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
        .keymap_picker_origin_scope
        .as_deref()
        .unwrap_or("unknown");
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

#[cfg(test)]
#[path = "picker_render_tests.rs"]
mod render_tests;
