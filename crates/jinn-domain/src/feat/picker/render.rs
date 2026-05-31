//! Picker rendering for persona and theme pickers.

use crate::common::app_state::AppState;
use crate::feat::ui::picker_states::PickerExt;
use jinn_selection_widget::PreviewSelectionWidget;
use jinn_selection_widget::SelectionWidget;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;

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
    let widget = SelectionWidget::new(state.frontend.persona_picker())
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
    let widget = SelectionWidget::new(state.frontend.theme_picker())
        .title(Line::from(" Themes "))
        .title_style(Style::default().fg(state.frontend.theme.popup_title))
        .footer(Line::from(" ESC to cancel, Enter to apply "));
    widget.render(frame, area);
}

/// Renders the workflow picker overlay using [`SelectionWidget`].
///
/// Telescope-style layout: bordered popup with filter input at top,
/// horizontal separator, scrollable workflow entries.
pub fn render_workflow_picker(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let widget = SelectionWidget::new(state.frontend.workflow_picker())
        .title(Line::from(" Workflows "))
        .title_style(Style::default().fg(state.frontend.theme.popup_title))
        .footer(Line::from(" Enter to run, ESC to cancel "));
    widget.render(frame, area);
}


/// Renders the tool picker overlay.
pub fn render_tool_picker(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let enabled_count = state
        .frontend
        .tool_picker()
        .items()
        .iter()
        .filter(|t| t.enabled)
        .count();
    let total = state.frontend.tool_picker().items().len();
    let footer = Line::from(format!(
        " TAB toggle \u{00b7} {enabled_count}/{total} enabled \u{00b7} Enter confirm \u{00b7} ESC cancel "
    ));
    let widget = SelectionWidget::new(state.frontend.tool_picker())
        .title(Line::from(" Tools "))
        .title_style(Style::default().fg(state.frontend.theme.popup_title))
        .footer(footer);
    widget.render(frame, area);
}

/// Renders the skill picker overlay with a preview pane.
///
/// Uses [`PreviewSelectionWidget`] to show the selected skill's markdown body
/// in a split pane (vertical on wide terminals, horizontal on narrow ones).
pub fn render_skill_picker(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let enabled_count = state
        .frontend
        .skill_picker()
        .items()
        .iter()
        .filter(|s| s.enabled)
        .count();
    let total = state.frontend.skill_picker().items().len();
    let footer = Line::from(format!(
        " TAB toggle \u{00b7} {enabled_count}/{total} enabled \u{00b7} Enter confirm \u{00b7} ESC cancel "
    ));
    let widget = PreviewSelectionWidget::new(state.frontend.skill_picker())
        .title(Line::from(" Skills "))
        .title_style(Style::default().fg(state.frontend.theme.popup_title))
        .preview_scroll(state.frontend.skill_preview_scroll())
        .footer(footer);
    widget.render(frame, area);
}
