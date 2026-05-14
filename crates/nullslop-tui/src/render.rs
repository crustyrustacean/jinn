//! Layout computation and rendering for the application.

pub mod app_layout;
pub mod clipboard;
pub mod picker;
pub mod selection_highlight;
pub mod tab_bar;
pub mod too_small;
pub mod which_key;

pub use app_layout::{AppLayout, MIN_HEIGHT, MIN_WIDTH};
pub use tab_bar::init_tab_manager;

use nullslop_domain::{FocusScope, Mode};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

use crate::TuiApp;

/// Renders the full application frame.
#[expect(clippy::too_many_lines, reason = "render dispatches to sub-functions but the match itself is long")]
pub fn render(app: &mut TuiApp, frame: &mut Frame<'_>) {
    let area = frame.area();
    if !AppLayout::meets_min_size(area) {
        too_small::render_too_small(frame, area, app);
        return;
    }

    // Pre-render mutation: set wrap width and scroll offset using a write lock.
    {
        let mut wstate = app.core.state.write();
        // Compute a preliminary layout to determine main column width.
        let max_input_height = area.height / 2;
        let pre_layout = AppLayout::new(
            area,
            wstate.active_chat_input().visual_line_count() as u16,
            max_input_height,
        );
        // Wrap width is based on the main column (not full terminal width).
        let text_width = pre_layout.main.width.saturating_sub(2) as usize;
        wstate.active_chat_input_mut().set_wrap_width(text_width);
        if wstate.frontend.scope_stack.current().mode() == Mode::Input {
            let inner_height = pre_layout.input.height.saturating_sub(2) as usize;
            wstate
                .active_chat_input_mut()
                .scroll_to_cursor(inner_height);
        }
    }

    let state = app.core.state.read();

    let max_input_height = area.height / 2;
    let layout = AppLayout::new(
        area,
        state.active_chat_input().visual_line_count() as u16,
        max_input_height,
    );

    // Tab bar — always visible.
    tab_bar::render_tab_bar(frame, layout.tabs, &app.tab_manager);

    // Collect selectable rects during rendering.
    let mut rects = vec![];

    match state.frontend.active_tab {
        nullslop_domain::ActiveTab::Chat => {
            // Draw vertical border line between main and sidebar.
            let border_color = if state.frontend.scope_stack.is_sidebar() {
                Color::Yellow
            } else {
                Color::DarkGray
            };
            let border_style = Style::default().fg(border_color);
            for y in layout.border.y..(layout.border.y + layout.border.height) {
                if let Some(cell) = frame.buffer_mut().cell_mut((layout.border.x, y)) {
                    cell.set_symbol("\u{2502}");
                    cell.set_style(border_style);
                }
            }

            // Render sidebar sections.
            let sidebar_focused = state.frontend.scope_stack.is_sidebar();
            app.sidebar.render(frame, layout.sidebar, &state);
            if sidebar_focused {
                rects.push(layout.sidebar);
            }

            // Use layout.content (main column sub-area) for the chat log.
            let content_area = layout.content;

            // Compute sub-areas at the bottom of the content area for
            // the streaming indicator, queue, and bottom line.
            let queue_len = state.active_session().queue_len() as u16;
            let bottom_lines = 1 + queue_len + 1; // indicator + queue + chat bottom line
            let chat_log_area = if content_area.height > bottom_lines {
                Rect {
                    x: content_area.x,
                    y: content_area.y,
                    width: content_area.width,
                    height: content_area.height - bottom_lines,
                }
            } else {
                // Not enough space — give everything to chat log.
                content_area
            };

            // Chat log
            if let Some(element) = app.ui_registry.get_mut("chat-log") {
                element.render(frame, chat_log_area, &state);
                if element.is_selectable() && !sidebar_focused {
                    rects.push(content_area);
                }
            }
            // Streaming indicator (1 row at bottom of content area)
            {
                // Always reserve indicator row position
                let indicator_y = content_area.y + content_area.height.saturating_sub(bottom_lines);
                let indicator_area = Rect {
                    x: content_area.x,
                    y: indicator_y,
                    width: content_area.width,
                    height: 1,
                };
                if let Some(element) = app.ui_registry.get_mut("streaming-indicator") {
                    element.render(frame, indicator_area, &state);
                }
            }
            // Queue display (dynamic rows)
            if queue_len > 0 {
                let queue_y = content_area.y + content_area.height.saturating_sub(bottom_lines) + 1;
                let queue_area = Rect {
                    x: content_area.x,
                    y: queue_y,
                    width: content_area.width,
                    height: queue_len,
                };
                if let Some(element) = app.ui_registry.get_mut("queue-display") {
                    element.render(frame, queue_area, &state);
                }
            }
            // Chat bottom line — horizontal separator at the bottom of content area
            let line_y = content_area.y + content_area.height.saturating_sub(1);
            let chat_line_color =
                if matches!(state.frontend.scope_stack.current(), FocusScope::Normal) {
                    Color::Yellow
                } else {
                    Color::DarkGray
                };
            let chat_line_style = Style::default().fg(chat_line_color);
            for x in content_area.x..(content_area.x + content_area.width) {
                if let Some(cell) = frame.buffer_mut().cell_mut((x, line_y)) {
                    cell.set_symbol("\u{2500}");
                    cell.set_style(chat_line_style);
                }
            }
            // Input box
            if let Some(element) = app.ui_registry.get_mut("chat-input-box") {
                element.render(frame, layout.input, &state);
            }

            // Autocomplete popup overlay (transient, not a UiElement).
            if state.active_chat_input().autocomplete().is_some() {
                nullslop_domain::feat::chat_input::autocomplete_render::render_autocomplete_popup(
                    frame,
                    layout.input,
                    &state,
                );
            }
        }
        nullslop_domain::ActiveTab::Dashboard => {
            // Dashboard fills the entire content area
            if let Some(element) = app.ui_registry.get_mut("dashboard") {
                element.render(frame, layout.content, &state);
                if element.is_selectable() {
                    rects.push(layout.content);
                }
            }
        }
    }

    // Status bar — always visible at bottom.
    if let Some(element) = app.ui_registry.get_mut("status-bar") {
        element.render(frame, layout.status_bar, &state);
    }

    // Which-key popup overlay (app-level, not a component element)
    which_key::render_which_key(frame, &mut app.which_key);

    if state.frontend.scope_stack.is_picker() {
        picker::render_picker(frame, area, &state);
        // Provider picker popup is selectable — not a UiElement, register inline.
        rects.push(nullslop_selection_widget::compute_popup_rect(area));
    }

    // Release the state read lock before clipboard flush needs &mut app.
    drop(state);

    app.selectable_rects.rebuild(rects);

    // Apply selection highlight after all elements have rendered.
    selection_highlight::apply_selection_highlight(app, frame.buffer_mut());

    // Flush pending clipboard copy (reads buffer, writes system clipboard).
    clipboard::flush_pending_clipboard(app, frame.buffer_mut());
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod render_tests;
