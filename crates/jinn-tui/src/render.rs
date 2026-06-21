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

use jinn_domain::{AppUiRegistry, FocusScope, Mode, RenderCtx, feat::ui::sidebar::Sidebar};
use ratatui::{Frame, layout::Rect};

use crate::TuiApp;
use crate::app::WhichKeyInstance;

/// Renders the full application frame.
pub fn render(app: &mut TuiApp, frame: &mut Frame<'_>) {
    let area = frame.area();
    if !AppLayout::meets_min_size(area) {
        too_small::render_too_small(frame, area, app);
        return;
    }

    apply_pre_render_mutation(app, area);

    let state = app.core.state.read();
    let ctx = RenderCtx::new(&state).with_plugins(&app.plugins);

    let layout = AppLayout::new(
        area,
        state.active_chat_input().visual_line_count() as u16,
        area.height / 2,
        state.frontend.sidebar_width,
    );
    let sidebar_focused = state.frontend.scope_stack.is_sidebar();
    let active_scope = state.frontend.scope_stack.current();

    let mut rects = vec![];
    render_base_layers(
        &mut app.sidebar,
        &mut app.ui_registry,
        &mut app.which_key,
        frame,
        &ctx,
        &layout,
        area,
        sidebar_focused,
        &mut rects,
    );
    if let Some(rect) = render_active_overlay(frame, area, &ctx, active_scope) {
        rects.push(rect);
    }

    drop(state);

    app.selectable_rects.rebuild(rects);
    selection_highlight::apply_selection_highlight(app, frame.buffer_mut());
    clipboard::flush_pending_clipboard(app, frame.buffer_mut());
}

/// Sets wrap width and scroll offset before layout, using a write lock.
fn apply_pre_render_mutation(app: &mut TuiApp, area: Rect) {
    let mut wstate = app.core.state.write();
    let pre_layout = AppLayout::new(
        area,
        wstate.active_chat_input().visual_line_count() as u16,
        area.height / 2,
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
    jinn_domain::feat::ui::sidebar::task_list_section::preview::write_preview_geometry(
        &mut wstate,
        area,
        pre_layout.sidebar,
    );
}

/// Renders the always-visible layers: border, sidebar, chat tab, session preview,
/// status bar, and which-key popup.
#[expect(
    clippy::too_many_arguments,
    reason = "all inputs are single-use render pass params"
)]
fn render_base_layers(
    sidebar: &mut Sidebar,
    ui_registry: &mut AppUiRegistry,
    which_key: &mut WhichKeyInstance,
    frame: &mut Frame<'_>,
    ctx: &RenderCtx<'_>,
    layout: &AppLayout,
    frame_area: Rect,
    sidebar_focused: bool,
    rects: &mut Vec<Rect>,
) {
    chat_tab::border::render_border(frame, layout.border, ctx);
    chat_tab::sidebar::render_sidebar(sidebar, frame, layout.sidebar, sidebar_focused, ctx, rects);
    chat_tab::render_chat_tab(ui_registry, frame, layout, ctx, rects);
    jinn_domain::feat::ui::sidebar::sessions::render_session_preview_for_state(
        frame,
        layout.sidebar,
        frame_area,
        ctx,
    );
    jinn_domain::feat::ui::sidebar::task_list_section::preview::render_task_list_preview_for_state(
        frame,
        layout.sidebar,
        frame_area,
        ctx,
    );
    status_bar::render_status_bar(ui_registry, frame, layout.status_bar, ctx);
    which_key::render_which_key(frame, which_key, ctx);
}

/// Renders the single popup matching the active scope, if any, and returns its
/// selectable rect so the caller can register it.
fn render_active_overlay(
    frame: &mut Frame<'_>,
    area: Rect,
    ctx: &RenderCtx<'_>,
    scope: &FocusScope,
) -> Option<Rect> {
    match scope {
        FocusScope::Picker { .. } => {
            picker::render_picker(frame, area, ctx);
            Some(jinn_selection_widget::compute_popup_rect(area))
        }
        FocusScope::ArgInput => {
            picker::render_arg_input(frame, area, ctx);
            Some(jinn_domain::feat::session_lifecycle::render::arg_input_popup_rect(area, ctx))
        }
        FocusScope::RenameSessionInput => {
            jinn_domain::feat::rename_session_input::render::render_rename_session_input(
                frame, area, ctx,
            );
            Some(jinn_domain::feat::rename_session_input::render::rename_session_popup_rect(area))
        }
        FocusScope::PrunerAccumulationInput => {
            jinn_domain::feat::pruner_accumulation_input::render::render_pruner_accumulation_input(
                frame, area, ctx,
            );
            Some(jinn_domain::feat::pruner_accumulation_input::render::pruner_accumulation_popup_rect(area))
        }
        FocusScope::CwdInput => {
            jinn_domain::feat::cwd_input::render::render_cwd_input(frame, area, ctx);
            Some(jinn_domain::feat::cwd_input::render::cwd_input_popup_rect(
                area,
            ))
        }
        FocusScope::ProjectAddInput => {
            jinn_domain::feat::project_add_input::render::render_project_add_input(
                frame, area, ctx,
            );
            Some(jinn_domain::feat::project_add_input::render::project_add_input_popup_rect(area))
        }
        FocusScope::QuakeBar => {
            jinn_domain::feat::quake_bar::render::render_quake_bar(frame, area, ctx);
            None
        }
        _ => None,
    }
}
