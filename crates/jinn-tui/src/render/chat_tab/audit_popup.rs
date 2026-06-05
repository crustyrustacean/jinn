//! Audit popup overlay - renders the context-change history of the
//! currently-selected chat entry.
//!
//! Activated by pressing `a` in Normal mode (toggles
//! `FrontendState::audit_popup_visible`). The popup is right-aligned to the
//! chat-log area and vertically anchored to the top of the selected entry.
//! It tracks the cursor live as the user navigates.

use jinn_domain::RenderCtx;
use jinn_domain::common::focus::FocusScope;
use jinn_domain::feat::ui::chat_log::audit_popup::{audit_popup_rect, format_audit_lines};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

/// Render the audit popup, if it should be visible.
///
/// Conditions for visibility:
/// - `audit_popup_visible` is true
/// - No higher-priority overlay is active
/// - An entry is selected
///
/// `chat_log_area` is the chat-log content area (right-aligned anchor and
// bottom-edge clamp).
pub(super) fn render_audit_popup(
    frame: &mut Frame<'_>,
    chat_log_area: Rect,
    ctx: &RenderCtx,
    rects: &mut Vec<Rect>,
) {
    if !ctx.state.frontend.audit_popup_visible {
        return;
    }

    // Suppress when a higher-priority overlay is active.
    if overlay_active(ctx) {
        return;
    }

    let Some(entry) = ctx.state.active_session().selected_entry() else {
        return;
    };

    let lines = format_audit_lines(entry, &ctx.state.frontend.theme);

    // Resolve the screen Y of the selected entry's top edge.
    // Returns None until the render pipeline has populated cached fields for
    // this frame; in that case we simply skip rendering for this frame.
    let Some(entry_top_y) = ctx
        .state
        .active_session()
        .selected_entry_screen_y(chat_log_area.y)
    else {
        return;
    };

    let rect = audit_popup_rect(chat_log_area, entry_top_y, lines.len());

    // Clear underlying buffer so the popup is opaque.
    frame.render_widget(Clear, rect);

    // Render the popup body. The block provides top+bottom borders; the
    // header line is the first body line (so it scrolls with content).
    let block = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .style(
            Style::default()
                .bg(ctx.state.frontend.theme.infopopup_bg)
                .fg(ctx.state.frontend.theme.infopopup_border),
        );
    let paragraph = Paragraph::new(lines).style(
        Style::default()
            .bg(ctx.state.frontend.theme.infopopup_bg)
            .fg(ctx.state.frontend.theme.infopopup_fg),
    );
    frame.render_widget(paragraph.block(block), rect);

    rects.push(rect);
}

/// Returns true if a higher-priority overlay is currently active.
fn overlay_active(ctx: &RenderCtx) -> bool {
    matches!(
        ctx.state.frontend.scope_stack.current(),
        FocusScope::Picker { .. }
            | FocusScope::ArgInput
            | FocusScope::RenameSessionInput
            | FocusScope::SidebarSessions
    )
}
