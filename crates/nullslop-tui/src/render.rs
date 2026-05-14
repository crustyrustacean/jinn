//! Layout computation and rendering for the application.

pub mod app_layout;
pub mod chat_tab;
pub mod clipboard;
pub mod picker;
pub mod selection_highlight;
pub mod tab_bar;
pub mod too_small;
pub mod which_key;

pub use app_layout::{AppLayout, MIN_HEIGHT, MIN_WIDTH};
pub use tab_bar::init_tab_manager;

use nullslop_domain::Mode;
use ratatui::Frame;

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
            chat_tab::render_chat_tab(
                &mut app.ui_registry,
                &mut app.sidebar,
                frame,
                &layout,
                &state,
                &mut rects,
            );
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
