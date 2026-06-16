//! Layout computation and rendering for the application.

pub mod app_layout;
pub mod chat_tab;
pub mod clipboard;
pub mod picker;
pub mod selection_highlight;
pub mod status_bar;

pub mod too_small;
pub mod which_key;

pub use app_layout::{AppLayout, MIN_HEIGHT, MIN_WIDTH};

use jinn_domain::{FocusScope, Mode, RenderCtx};
use ratatui::Frame;

use crate::TuiApp;

/// Renders the full application frame.
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
            wstate.frontend.sidebar_width,
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
    let ctx = RenderCtx::new(&state).with_plugins(&app.plugins);

    let max_input_height = area.height / 2;
    let layout = AppLayout::new(
        area,
        state.active_chat_input().visual_line_count() as u16,
        max_input_height,
        state.frontend.sidebar_width,
    );

    let mut rects = vec![];

    let sidebar_focused = state.frontend.scope_stack.is_sidebar();

    // Vertical border between main and sidebar.
    chat_tab::border::render_border(frame, layout.border, &ctx);

    // Sidebar - always rendered regardless of content view.
    chat_tab::sidebar::render_sidebar(
        &mut app.sidebar,
        frame,
        layout.sidebar,
        sidebar_focused,
        &ctx,
        &mut rects,
    );

    // Main content area - chat tab.
    {
        chat_tab::render_chat_tab(&mut app.ui_registry, frame, &layout, &ctx, &mut rects);
    }
    // Session preview popup - when sidebar sessions section is focused.
    jinn_domain::feat::ui::sidebar::sessions::render_session_preview_for_state(
        frame,
        layout.sidebar,
        area,
        &ctx,
    );

    // Status bar - always visible at bottom.
    status_bar::render_status_bar(&mut app.ui_registry, frame, layout.status_bar, &ctx);

    // Which-key popup overlay.
    which_key::render_which_key(frame, &mut app.which_key, &ctx);

    // Picker overlay + selectable rect.
    if state.frontend.scope_stack.is_picker() {
        picker::render_picker(frame, area, &ctx);
        rects.push(jinn_selection_widget::compute_popup_rect(area));
    }

    // Arg input popup overlay (+ selectable rect).
    if matches!(state.frontend.scope_stack.current(), FocusScope::ArgInput) {
        picker::render_arg_input(frame, area, &ctx);
        rects.push(jinn_domain::feat::session_lifecycle::render::arg_input_popup_rect(area, &ctx));
    }

    // Rename session input popup overlay (+ selectable rect).
    if matches!(
        state.frontend.scope_stack.current(),
        FocusScope::RenameSessionInput
    ) {
        jinn_domain::feat::rename_session_input::render::render_rename_session_input(
            frame, area, &ctx,
        );
        rects
            .push(jinn_domain::feat::rename_session_input::render::rename_session_popup_rect(area));
    }

    // Pruner accumulation threshold input popup overlay (+ selectable rect).
    if matches!(
        state.frontend.scope_stack.current(),
        FocusScope::PrunerAccumulationInput
    ) {
        jinn_domain::feat::pruner_accumulation_input::render::render_pruner_accumulation_input(
            frame, area, &ctx,
        );
        rects.push(
            jinn_domain::feat::pruner_accumulation_input::render::pruner_accumulation_popup_rect(
                area,
            ),
        );
    }

    // CWD input popup overlay (+ selectable rect).
    if matches!(state.frontend.scope_stack.current(), FocusScope::CwdInput) {
        jinn_domain::feat::cwd_input::render::render_cwd_input(frame, area, &ctx);
        rects.push(jinn_domain::feat::cwd_input::render::cwd_input_popup_rect(
            area,
        ));
    }

    // Quake bar overlay (last, so it covers everything below it).
    if matches!(state.frontend.scope_stack.current(), FocusScope::QuakeBar) {
        jinn_domain::feat::quake_bar::render::render_quake_bar(frame, area, &ctx);
    }

    drop(state);

    app.selectable_rects.rebuild(rects);
    selection_highlight::apply_selection_highlight(app, frame.buffer_mut());
    clipboard::flush_pending_clipboard(app, frame.buffer_mut());
}
