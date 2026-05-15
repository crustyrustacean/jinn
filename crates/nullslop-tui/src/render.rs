//! Layout computation and rendering for the application.

pub mod app_layout;
pub mod chat_tab;
pub mod clipboard;
pub mod dashboard_tab;
pub mod picker;
pub mod selection_highlight;
pub mod status_bar;
pub mod tab_bar;
pub mod too_small;
pub mod which_key;

pub use app_layout::{AppLayout, MIN_HEIGHT, MIN_WIDTH};
pub use tab_bar::init_tab_manager;

use nullslop_domain::Mode;
use ratatui::Frame;

use crate::TuiApp;

/// Renders the full application frame.
#[expect(
    clippy::too_many_lines,
    reason = "render dispatches to sub-functions but the match itself is long"
)]
pub fn render(app: &mut TuiApp, frame: &mut Frame<'_>) {
    let area = frame.area();
    if !AppLayout::meets_min_size(area) {
        too_small::render_too_small(frame, area, app);
        return;
    }

    // Pre-render mutation: set wrap width and scroll offset using a write lock.
    {
        let mut wstate = app.core.state.write();
        let max_input_height = area.height / 2;
        let pre_layout = AppLayout::new(
            area,
            wstate.active_chat_input().visual_line_count() as u16,
            max_input_height,
        );
        let text_width = pre_layout.main.width.saturating_sub(2) as usize;
        wstate.active_chat_input_mut().set_wrap_width(text_width);
        if wstate.frontend.scope_stack.current().mode() == Mode::Input {
            let inner_height = pre_layout.input.height.saturating_sub(1) as usize;
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
    {
        let theme = &state.frontend.theme;
        tab_bar::render_tab_bar(
            frame,
            layout.tabs,
            &app.tab_manager,
            theme.tab_active_fg,
            theme.tab_active_bg,
            theme.tab_inactive_fg,
        );
    }

    // Active tab content.
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
            dashboard_tab::render_dashboard_tab(
                &mut app.ui_registry,
                frame,
                layout.content,
                &state,
                &mut rects,
            );
        }
    }

    // Status bar — always visible at bottom.
    status_bar::render_status_bar(&mut app.ui_registry, frame, layout.status_bar, &state);

    // Which-key popup overlay.
    {
        let theme = &state.frontend.theme;
        which_key::render_which_key(frame, &mut app.which_key, theme.focus_accent);
    }

    // Picker overlay + selectable rect.
    if state.frontend.scope_stack.is_picker() {
        picker::render_picker(frame, area, &state);
        rects.push(nullslop_selection_widget::compute_popup_rect(area));
    }

    // Release the state read lock before post-render steps.
    drop(state);

    app.selectable_rects.rebuild(rects);
    selection_highlight::apply_selection_highlight(app, frame.buffer_mut());
    clipboard::flush_pending_clipboard(app, frame.buffer_mut());
}
