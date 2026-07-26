//! Rendering for the MCP server inspector picker overlay.

use crate::RenderCtx;
use crate::feat::ui::picker_states::PickerExt;
use jinn_selection_widget::PreviewSelectionWidget;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

/// Renders the MCP server inspector overlay.
///
/// Multipane: the server list (left) with a preview pane (right) that shows
/// either the selected server's live status + stderr tail (logs mode, default)
/// or its advertised tools (tools mode). Mirrors the skills picker layout.
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

    let gray = Style::default().fg(state.frontend.theme.muted_text);
    let orange = Style::default().fg(state.frontend.theme.accent_action);
    let footer = Line::from(vec![
        Span::styled("TAB ".to_owned(), orange),
        Span::styled("toggle · ".to_owned(), gray),
        Span::styled("CTRL+R ".to_owned(), orange),
        Span::styled("restart · ".to_owned(), gray),
        Span::styled("CTRL+T ".to_owned(), orange),
        Span::styled("logs/tools · ".to_owned(), gray),
        Span::styled(
            format!("{enabled_count}/{total} enabled · Enter confirm · ESC cancel"),
            gray,
        ),
    ]);

    // MCP preview is live (status/stderr/tools refresh every frame), so no cache.
    let widget = PreviewSelectionWidget::new(state.frontend.mcp_server_picker())
        .title(Line::from(" MCP Servers "))
        .title_style(Style::default().fg(state.frontend.theme.popup_title))
        .footer(footer);
    widget.render(frame, area);
}
