//! Rendering for the MCP server picker overlay.

use crate::RenderCtx;
use crate::feat::ui::picker_states::PickerExt;
use jinn_selection_widget::SelectionWidget;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;

/// Renders the MCP server picker overlay.
///
/// Lists configured MCP servers with a toggle marker; TAB toggles, Enter
/// commits the enabled set, ESC reverts. Mirrors the tool/skill picker layout.
pub fn render_mcp_server_picker(frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
    let state = ctx.state;
    let enabled_count = state
        .frontend
        .mcp_server_picker()
        .items()
        .iter()
        .filter(|s| s.enabled)
        .count();
    let total = state.frontend.mcp_server_picker().items().len();
    let footer = Line::from(format!(
        " TAB toggle \u{00b7} {enabled_count}/{total} enabled \u{00b7} Enter confirm \u{00b7} ESC cancel "
    ));
    let widget = SelectionWidget::new(state.frontend.mcp_server_picker())
        .title(Line::from(" MCP Servers "))
        .title_style(ratatui::style::Style::default().fg(state.frontend.theme.popup_title))
        .footer(footer);
    widget.render(frame, area);
}
