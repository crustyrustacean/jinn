//! Chat tab rendering - dispatches to individual chat sub-components.

#[cfg(test)]
mod audit_popup_tests;
pub mod border;
pub mod chat_bottom_line;

use jinn_kernel::RenderCtx;
use jinn_slices::{DrawTarget, Region};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use jinn_chat_log_view::chat_log::GUTTER_WIDTH;

use super::app_layout::AppLayout;
use super::region_dispatch::draw_region;

/// Renders the Chat tab content - chat log, streaming indicator,
/// queue, bottom line, input box, and autocomplete popup.
///
/// Called from the top-level renderer when the chat view is active.
/// Does NOT render the sidebar or border (those are rendered at top level).
/// Computes sub-areas from the layout and hands each one to the slice
/// that registered a draw function for it.
/// Selectable rects are collected into `rects` for mouse selection support.
pub(super) fn render_chat_tab(
    slices: &jinn_slices::Slices,
    frame: &mut Frame<'_>,
    layout: &AppLayout,
    ctx: &RenderCtx,
    rects: &mut Vec<Rect>,
) {
    let sidebar_focused = ctx.state.frontend.is_sidebar();

    // Compute sub-areas at the bottom of the content area.
    let content_area = layout.content;
    let bottom_lines = 2; // indicator + chat bottom line

    let chat_log_area = if content_area.height > bottom_lines {
        Rect {
            x: content_area.x,
            y: content_area.y,
            width: content_area.width,
            height: content_area.height - bottom_lines,
        }
    } else {
        content_area
    };

    // Chat log. The chat-log slice draws it and reports whether the
    // region supports selection; whether that selection is *registered*
    // also depends on the sidebar not holding focus, which is
    // composition's call because the sidebar is a column of this layout.
    let log_selection = if sidebar_focused {
        None
    } else {
        // The log paints into `chat_log_area`, but the selectable region
        // spans the full content area: the input box and the bottom line
        // below the log are not part of the selection.
        Some(Rect {
            x: content_area.x + GUTTER_WIDTH,
            y: content_area.y,
            width: content_area.width.saturating_sub(GUTTER_WIDTH),
            height: content_area.height,
        })
    };
    draw_region(
        slices,
        Region::ChatLog,
        frame,
        DrawTarget::with_select(chat_log_area, log_selection),
        ctx,
        rects,
    );

    // Audit popup overlay - renders above the chat log when toggled on.
    // The chat-log slice owns its visibility and its geometry, so it is
    // asked for by region rather than called by name.
    draw_region(
        slices,
        Region::AuditPopup,
        frame,
        DrawTarget::new(chat_log_area),
        ctx,
        rects,
    );

    // The vertical minimap column and the `>` arrow that points at the
    // selected entry. The chat-log slice reads the same session history
    // the log does, so it owns both.
    draw_region(
        slices,
        Region::Minimap,
        frame,
        DrawTarget::with_select(layout.minimap, Some(chat_log_area)),
        ctx,
        rects,
    );

    // Streaming indicator.
    let indicator_y = content_area.y + content_area.height.saturating_sub(bottom_lines);
    let indicator_area = Rect {
        x: content_area.x,
        y: indicator_y,
        width: content_area.width,
        height: 1,
    };
    draw_region(
        slices,
        Region::StreamingIndicator,
        frame,
        DrawTarget::new(indicator_area),
        ctx,
        rects,
    );

    // Cancel stream prompt - overlay at bottom of chat log area. This is
    // composition's own chrome: it is a global confirmation bound to a
    // frontend flag, not any slice's content.
    //
    // Shown only while something a cancel would reach is running. The flag can
    // outlive that work — nothing clears it when a turn simply finishes, and no
    // keystroke is involved when that happens — so the bar would sit there
    // offering to cancel work that has already ended. The kernel owns the
    // condition, so calling it here is what keeps the bar from appearing over a
    // session the handler would decline to cancel.
    let cancellable = jinn_kernel::feat::intent::cancel::subtree_has_running_work(
        ctx.state,
        ctx.state.session.active_session_id(),
    );
    if ctx.state.frontend.cancel_stream_prompt && cancellable {
        let prompt_area = Rect {
            x: chat_log_area.x,
            y: chat_log_area.y + chat_log_area.height.saturating_sub(1),
            width: chat_log_area.width,
            height: 1,
        };
        let prompt = Paragraph::new(Line::from(Span::styled(
            " Press ESC again to cancel ",
            Style::default().fg(Color::Black).bg(Color::Yellow),
        )));
        frame.render_widget(prompt, prompt_area);
    }

    // Chat bottom line.
    chat_bottom_line::render_chat_bottom_line(frame, content_area, ctx);

    // Input box. The autocomplete popup is part of the same region: the
    // chat-input slice draws both, because the popup is anchored to the
    // box and exists only while it is focused.
    draw_region(
        slices,
        Region::ChatInput,
        frame,
        DrawTarget::new(layout.input),
        ctx,
        rects,
    );
}
