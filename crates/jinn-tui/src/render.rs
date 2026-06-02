//! Layout computation and rendering for the application.

pub mod app_layout;
pub mod chat_tab;
pub mod clipboard;
pub mod picker;
pub mod selection_highlight;
pub mod status_bar;

pub mod too_small;
pub mod which_key;
pub mod workflow_tab;

pub use app_layout::{AppLayout, MIN_HEIGHT, MIN_WIDTH};

use jinn_domain::{FocusScope, Mode};
use ratatui::Frame;

use jinn_domain::RenderCtx;
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

        // Pre-render mutation for workflow input buffer.
        prepare_workflow_input_scroll(&mut wstate, area, &pre_layout);
    }

    let state = app.core.state.read();
    let ctx = RenderCtx::new(&state);

    let max_input_height = area.height / 2;
    let layout = AppLayout::new(
        area,
        state.active_chat_input().visual_line_count() as u16,
        max_input_height,
        state.frontend.sidebar_width,
    );

    let mut rects = vec![];



    let sidebar_focused = state.frontend.scope_stack.is_sidebar();
    let focus_scope = state.frontend.scope_stack.current();

    // Vertical border between main and sidebar.
    let theme = &state.frontend.theme;
    chat_tab::border::render_border(
        frame,
        layout.border,
        focus_scope,
        theme.focus_accent,
        theme.border_unfocused,
        theme.sidebar_resize_accent,
    );

    // Sidebar - always rendered regardless of content view.
    chat_tab::sidebar::render_sidebar(
        &mut app.sidebar,
        frame,
        layout.sidebar,
        sidebar_focused,
        &state,
        &mut rects,
    );

    // Main content area - chat, workflow, or workflow preview.
    if state.is_viewing_workflow() {
        workflow_tab::render_workflow_tab(frame, layout.content, &state, 0);
    } else if let Some(workflow) = state.previewed_workflow() {
        workflow_tab::render_workflow_preview(frame, layout.content, workflow);
    } else {
        chat_tab::render_chat_tab(
            &mut app.ui_registry,
            frame,
            &layout,
            &state,
            &mut rects,
        );
    }
    // Session preview popup - when sidebar sessions section is focused.
    jinn_domain::feat::ui::sidebar::sessions::render_session_preview_for_state(
        frame,
        layout.sidebar,
        area,
        &state,
    );

    // Status bar - always visible at bottom.
    status_bar::render_status_bar(&mut app.ui_registry, frame, layout.status_bar, &state);

    // Which-key popup overlay.
    {
        let theme = &state.frontend.theme;
        which_key::render_which_key(frame, &mut app.which_key, theme.focus_accent);
    }

    // Picker overlay + selectable rect.
    if state.frontend.scope_stack.is_picker() {
        picker::render_picker(frame, area, &state);
        rects.push(jinn_selection_widget::compute_popup_rect(area));
    }

    // Arg input popup overlay (+ selectable rect).
    if matches!(state.frontend.scope_stack.current(), FocusScope::ArgInput) {
        picker::render_arg_input(frame, area, &state);
        rects
            .push(jinn_domain::feat::session_lifecycle::render::arg_input_popup_rect(area, &state));
    }

    // Rename session input popup overlay (+ selectable rect).
    if matches!(
        state.frontend.scope_stack.current(),
        FocusScope::RenameSessionInput
    ) {
        jinn_domain::feat::rename_session_input::render::render_rename_session_input(
            frame, area, &state,
        );
        rects
            .push(jinn_domain::feat::rename_session_input::render::rename_session_popup_rect(area));
    }

    // Rename workflow input popup overlay (+ selectable rect).
    if matches!(
        state.frontend.scope_stack.current(),
        FocusScope::RenameWorkflowInput
    ) {
        jinn_domain::feat::rename_workflow_input::render::render_rename_workflow_input(
            frame, area, &state,
        );
        rects
            .push(jinn_domain::feat::rename_workflow_input::render::rename_workflow_popup_rect(area));
    }

    // Release the state read lock before post-render steps.
    drop(state);

    app.selectable_rects.rebuild(rects);
    selection_highlight::apply_selection_highlight(app, frame.buffer_mut());
    clipboard::flush_pending_clipboard(app, frame.buffer_mut());
}

/// Pre-render mutation for the workflow input buffer.
///
/// Sets wrap width and scroll offset so that `wrapped_lines()`, `scroll_offset()`,
/// and `cursor_row_col()` return correct values during the read-only render pass.
/// Mirrors the chat input pattern in `render()`.
fn prepare_workflow_input_scroll(
    wstate: &mut jinn_domain::AppState,
    area: ratatui::layout::Rect,
    pre_layout: &AppLayout,
) {
    if wstate.frontend.workflow_ui.editing_node.is_none() {
        return;
    }
    let input_height: u16 = 5; // border (1) + 3 visible lines + padding
    let content_height = area.height.saturating_sub(pre_layout.status_bar.height);
    let graph_height = content_height
        .saturating_sub(input_height)
        .max(content_height / 2);
    let actual_input_height = content_height.saturating_sub(graph_height);
    // Borders::TOP consumes 1 row, indent is 2 chars.
    let inner_height = actual_input_height.saturating_sub(1) as usize;
    let text_width = area.width.saturating_sub(2) as usize; // 2-char indent
    wstate
        .frontend
        .workflow_ui
        .input_buffer
        .set_wrap_width(text_width);
    if wstate.frontend.scope_stack.current().mode() == Mode::Input {
        wstate
            .frontend
            .workflow_ui
            .input_buffer
            .scroll_to_cursor(inner_height);
    }
}
