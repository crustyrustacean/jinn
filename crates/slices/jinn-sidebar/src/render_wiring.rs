//! The sidebar's slice-owned rendering registrations.
//!
//! The chat layout draws a border between the main column and the
//! sidebar, and a bottom line under the content. Both lines change
//! colour with whatever owns the focused scope — the sidebar claims
//! the focused accent, the sidebar's *resize* mode claims its own
//! transient accent, and every other scope leaves the chrome unfocused.
//!
//! Those are the sidebar's own rules, so the sidebar states them here,
//! once, at activation. The chrome reads a registered hint instead of
//! matching the string `"sidebar"` against a scope id.
//!
//! The three late overlays the sidebar paints over the chat column —
//! the archive-tree prompt, the close-session prompt, and the session
//! preview — register as one draw function against [`Region::Sidebar`]
//! too, so the render pass has a single sidebar call site.

use std::sync::Arc;

use jinn_kernel::common::app_state::AppState;
use jinn_slices::DrawContext;
use jinn_slices::pre_render::{ChatRects, PreRenderCtx};
use jinn_slices::scope_hints::{ScopeHints, ScopeRenderHint};
use jinn_slices::slice_scope::SliceScopeId;
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::sections::section_trait::SidebarSectionId;

/// Registers the render hints for every scope the sidebar owns.
///
/// Two rules, stated as data:
///
/// * Each section's navigation scope claims the chrome's *focused*
///   accent, because the sidebar is the focus owner while one of them
///   is active.
/// * The resize scope claims the *acting* accent, which is the
///   transient colour a drag uses. It is a different visual state from
///   merely being focused, and the sidebar is the only slice that
///   knows that.
/// * The rename popup's scope stands a lower overlay down, because
///   the rename input is modal over the sessions list and the chat
///   log's audit popup must not paint through it.
pub fn register_hints(hints: &ScopeHints) {
    for section in [
        SidebarSectionId::Pins,
        SidebarSectionId::Persona,
        SidebarSectionId::TaskList,
        SidebarSectionId::McpServers,
    ] {
        hints.register(section.scope_id(), ScopeRenderHint::focused());
    }
    hints.register(resize_scope(), ScopeRenderHint::acting());
    // Two scopes stand a lower overlay down, and they stand it for
    // different reasons, so each claims it rather than the chrome
    // listing sidebar scope names.
    hints.register(
        SidebarSectionId::Sessions.scope_id(),
        // The sessions list owns a rename input that paints over the
        // chat log, so the chat log's audit popup yields to it.
        ScopeRenderHint::focused().suppressing_lower_overlay(),
    );
    hints.register(
        rename_scope(),
        // The rename popup is modal over the sessions list.
        ScopeRenderHint::focused().suppressing_lower_overlay(),
    );
}

/// The sidebar's resize-mode scope.
fn resize_scope() -> SliceScopeId {
    jinn_sidebar_msg::SidebarSectionId::resize_scope_id()
}

/// The rename popup's scope.
fn rename_scope() -> SliceScopeId {
    crate::key_routes::rename_scope()
}

/// The sidebar's per-frame write hooks, in the order the render pass
/// runs them.
///
/// The pre-render pass holds one write lock and drives these in
/// sequence, so the order is part of the contract: the preview width
/// is recorded before anything reads it, and the scroll offset is
/// written after the geometry that depends on it.
pub fn register_pre_render_hooks(hooks: &jinn_slices::pre_render::PreRenderHooks<AppState>) {
    hooks.push(Arc::new(record_preview_width));
    hooks.push(Arc::new(request_session_preview));
    hooks.push(Arc::new(write_task_list_geometry));
    hooks.push(Arc::new(write_scroll_offset));
}

/// Records the width a session preview wraps its lines at.
///
/// The preview's render pass and the keyboard path must agree on the
/// width or the rendered lines can never match the lookup and the
/// preview spins forever. It is recorded here, in the one pass that
/// runs every frame with the true area and no early return —
/// recording it from the preview's own draw would leave the window
/// between a cursor move and the next frame asking for a stale width.
fn record_preview_width(
    state: &mut AppState,
    ctx: &PreRenderCtx<'_>,
) -> Vec<jinn_slices::route::PublishClosure> {
    let width = crate::sections::sessions::preview::preview_content_width(ctx.frame_area);
    state
        .frontend
        .update_sections(|s| s.sessions.preview_content_width = width);
    Vec::new()
}

/// Requests a session preview when the cache is cold and nothing is in
/// flight.
///
/// The popup's cache is not driven by the cursor alone: a session can
/// load into the list, or the width can be measured, long after the
/// last key. A cache miss with nothing in flight is a request nobody
/// made, so this pass makes it. `update_preview` dedupes against the
/// cache and against in-flight renders, so a settled cursor publishes
/// once and then stays silent.
fn request_session_preview(
    state: &mut AppState,
    ctx: &PreRenderCtx<'_>,
) -> Vec<jinn_slices::route::PublishClosure> {
    let Some(request) =
        crate::sections::sessions::preview_load::request_preview_if_needed(state, ctx.config)
    else {
        return Vec::new();
    };
    // The request and its deadline travel together, exactly as the
    // keyboard path sends them: a render nobody watches is a spinner
    // with nothing to end it.
    crate::sections::sessions::navigate::preview_messages(request).messages
}

/// Measures the task-list preview popup and writes its geometry into
/// state, so the scroll intents can page by a full viewport and clamp
/// without re-wrapping. A tab layout has no sidebar column, so the
/// hook stands down.
fn write_task_list_geometry(
    state: &mut AppState,
    ctx: &PreRenderCtx<'_>,
) -> Vec<jinn_slices::route::PublishClosure> {
    let Some(ChatRects { sidebar, .. }) = ctx.chat else {
        return Vec::new();
    };
    crate::sections::task_list_section::preview::write_preview_geometry(
        state,
        ctx.config,
        ctx.frame_area,
        sidebar,
    );
    Vec::new()
}

/// Records the sidebar's scroll offset for the frame about to render.
///
/// The offset is a pure function of the cursor, so there is nothing to
/// recompute here — this only remembers it, which is what lets the
/// column hold its position across a focus change to the chat pane
/// (where no cursor can be derived at all).
fn write_scroll_offset(
    state: &mut AppState,
    ctx: &PreRenderCtx<'_>,
) -> Vec<jinn_slices::route::PublishClosure> {
    let Some(ChatRects { sidebar, .. }) = ctx.chat else {
        return Vec::new();
    };
    crate::sections::layout::write_scroll_offset(state, ctx.config, sidebar.height);
    Vec::new()
}

/// The sidebar's late overlays: the archive-tree prompt, the
/// close-session prompt, and the session preview popup.
///
/// These paint over the chat column *after* the main column has
/// rendered, so a banner may extend left across the input box. They
/// are anchored to the sidebar's own rect, which is why they belong
/// to the sidebar and not to the chat layout that provides the space.
pub fn draw_late_overlays(
    frame: &mut Frame<'_>,
    sidebar_rect: Rect,
    frame_area: Rect,
    ctx: &dyn DrawContext<AppState>,
) {
    crate::sections::sessions::render_archive_tree_prompt_for_state(
        frame,
        sidebar_rect,
        frame_area,
        ctx,
    );
    crate::sections::sessions::render_close_session_prompt_for_state(
        frame,
        sidebar_rect,
        frame_area,
        ctx,
    );
    crate::sections::sessions::render_session_preview_for_state(
        frame,
        sidebar_rect,
        frame_area,
        ctx,
    );
    crate::sections::task_list_section::preview::render_task_list_preview_for_state(
        frame,
        sidebar_rect,
        frame_area,
        ctx,
    );
}
