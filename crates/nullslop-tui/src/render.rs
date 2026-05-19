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

use std::time::Instant;

use nullslop_domain::{FocusScope, Mode};
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
    let write_lock_start = Instant::now();
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

    let write_lock_dur = write_lock_start.elapsed();

    let read_lock_start = Instant::now();
    let state = app.core.state.read();
    let read_lock_dur = read_lock_start.elapsed();

    let max_input_height = area.height / 2;
    let layout = AppLayout::new(
        area,
        state.active_chat_input().visual_line_count() as u16,
        max_input_height,
        state.frontend.sidebar_width,
    );

    // Tab bar removed — content renders directly.
    let mut rects = vec![];
    chat_tab::render_chat_tab(
        &mut app.ui_registry,
        &mut app.sidebar,
        frame,
        &layout,
        &state,
        &mut rects,
    );

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

    // Arg input popup overlay (+ selectable rect).
    if matches!(state.frontend.scope_stack.current(), FocusScope::ArgInput) {
        picker::render_arg_input(frame, area, &state);
        rects.push(
            nullslop_domain::feat::session_lifecycle::render::arg_input_popup_rect(area, &state),
        );
    }

    // Token budget input popup overlay (+ selectable rect).
    if matches!(
        state.frontend.scope_stack.current(),
        FocusScope::TokenBudgetInput
    ) {
        nullslop_domain::feat::token_budget_input::render::render_token_budget_input(
            frame, area, &state,
        );
        rects
            .push(nullslop_domain::feat::token_budget_input::render::token_budget_popup_rect(area));
    }

    // Sliding window input popup overlay (+ selectable rect).
    if matches!(
        state.frontend.scope_stack.current(),
        FocusScope::SlidingWindowInput
    ) {
        nullslop_domain::feat::sliding_window_input::render::render_sliding_window_input(
            frame, area, &state,
        );
        rects.push(
            nullslop_domain::feat::sliding_window_input::render::sliding_window_popup_rect(area),
        );
    }

    // Rename session input popup overlay (+ selectable rect).
    if matches!(
        state.frontend.scope_stack.current(),
        FocusScope::RenameSessionInput
    ) {
        nullslop_domain::feat::rename_session_input::render::render_rename_session_input(
            frame, area, &state,
        );
        rects.push(
            nullslop_domain::feat::rename_session_input::render::rename_session_popup_rect(area),
        );
    }

    // Release the state read lock before post-render steps.
    drop(state);

    let read_hold_dur = read_lock_start.elapsed();

    let post_render_start = Instant::now();
    app.selectable_rects.rebuild(rects);
    selection_highlight::apply_selection_highlight(app, frame.buffer_mut());
    clipboard::flush_pending_clipboard(app, frame.buffer_mut());
    let post_render_dur = post_render_start.elapsed();

    let total_render = write_lock_start.elapsed();
    let write_us = write_lock_dur.as_micros() as u64;
    let read_wait_us = read_lock_dur.as_micros() as u64;
    let read_hold_us = read_hold_dur.as_micros() as u64;
    let post_us = post_render_dur.as_micros() as u64;
    let total_us = total_render.as_micros() as u64;
    if total_us > 10_000 {
        tracing::warn!(
            write_us,
            read_wait_us,
            read_hold_us,
            post_us,
            total_us,
            "PERF: render slow (>10ms)"
        );
    }
}
