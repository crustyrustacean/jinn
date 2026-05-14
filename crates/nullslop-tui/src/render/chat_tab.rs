//! Chat tab rendering — dispatches to individual chat sub-components.

pub mod autocomplete;
pub mod border;
pub mod chat_bottom_line;
pub mod chat_log;
pub mod input_box;
pub mod queue_display;
pub mod sidebar;
pub mod streaming_indicator;

use nullslop_domain::AppState;
use nullslop_domain::AppUiRegistry;
use nullslop_domain::feat::ui::sidebar::Sidebar;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::app_layout::AppLayout;

/// Renders the full Chat tab — border, sidebar, chat log, streaming indicator,
/// queue, bottom line, input box, and autocomplete popup.
///
/// Computes sub-areas from the layout and delegates to individual render functions.
/// Selectable rects are collected into `rects` for mouse selection support.
pub(super) fn render_chat_tab(
    ui_registry: &mut AppUiRegistry,
    sidebar: &mut Sidebar,
    frame: &mut Frame<'_>,
    layout: &AppLayout,
    state: &AppState,
    rects: &mut Vec<Rect>,
) {
    let sidebar_focused = state.frontend.scope_stack.is_sidebar();
    let focus_scope = state.frontend.scope_stack.current();

    // Vertical border between main and sidebar.
    border::render_border(frame, layout.border, sidebar_focused);

    // Sidebar.
    sidebar::render_sidebar(
        sidebar,
        frame,
        layout.sidebar,
        sidebar_focused,
        state,
        rects,
    );

    // Compute sub-areas at the bottom of the content area.
    let content_area = layout.content;
    let queue_len = state.active_session().queue_len() as u16;
    let bottom_lines = 2; // indicator + chat bottom line (queue overlays chat log)

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

    // Chat log.
    chat_log::render_chat_log(
        ui_registry,
        frame,
        chat_log_area,
        content_area,
        sidebar_focused,
        state,
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
    streaming_indicator::render_streaming_indicator(ui_registry, frame, indicator_area, state);

    // Queue display — rendered as overlay anchored at bottom of chat log area.
    // This paints over the last N lines of the chat log instead of pushing
    // the chat log up.
    if queue_len > 0 {
        let queue_area = Rect {
            x: chat_log_area.x,
            y: chat_log_area.y + chat_log_area.height.saturating_sub(queue_len),
            width: chat_log_area.width,
            height: queue_len,
        };
        queue_display::render_queue_display(ui_registry, frame, queue_area, state);
    }

    // Cancel stream prompt — overlay at bottom of chat log area.
    // Paints over whatever is behind it (including the queue display).
    if state.frontend.cancel_stream_prompt {
        let prompt_area = Rect {
            x: chat_log_area.x,
            y: chat_log_area.y + chat_log_area.height.saturating_sub(1),
            width: chat_log_area.width,
            height: 1,
        };
        let prompt = Paragraph::new(Line::from(Span::styled(
            "Press ESC again to cancel",
            Style::default().fg(Color::Yellow),
        )));
        frame.render_widget(prompt, prompt_area);
    }

    // Chat bottom line.
    chat_bottom_line::render_chat_bottom_line(frame, content_area, focus_scope);

    // Input box.
    input_box::render_input_box(ui_registry, frame, layout.input, state);

    // Autocomplete popup.
    autocomplete::render_autocomplete(frame, layout.input, state);
}
