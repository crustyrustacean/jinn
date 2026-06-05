//! Picker overlay rendering - dispatches to domain-specific picker renderers.

use jinn_domain::PickerKind;
use jinn_domain::RenderCtx;
use ratatui::Frame;
use ratatui::layout::Rect;

/// Renders the active picker overlay, dispatching on [`PickerKind`].
pub(super) fn render_picker(frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
    match ctx.state.frontend.scope_stack.picker_kind().copied() {
        Some(PickerKind::Provider) => render_provider_picker(frame, area, ctx),
        Some(PickerKind::Session) => render_session_picker(frame, area, ctx),
        Some(PickerKind::Persona) => render_persona_picker(frame, area, ctx),
        Some(PickerKind::Theme) => render_theme_picker(frame, area, ctx),
        Some(PickerKind::SessionLifecycle) => {
            render_session_lifecycle_picker(frame, area, ctx);
        }
        Some(PickerKind::Workflow) => {
            render_workflow_picker(frame, area, ctx);
        }

        Some(PickerKind::CompactionModel) => {
            jinn_domain::feat::provider::render::render_compaction_model_picker(frame, area, ctx);
        }
        Some(PickerKind::Tool) => {
            jinn_domain::feat::picker::render::render_tool_picker(frame, area, ctx);
        }
        Some(PickerKind::Skill) => {
            jinn_domain::feat::picker::render::render_skill_picker(frame, area, ctx);
        }
        Some(PickerKind::TaskList) => {
            jinn_domain::feat::picker::render::render_task_list_picker(frame, area, ctx);
        }
        None => {}
    }
}

/// Renders the provider picker overlay (delegates to slice).
fn render_provider_picker(frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
    jinn_domain::feat::provider::render::render_provider_picker(frame, area, ctx);
}

/// Renders the session picker overlay (delegates to slice).
fn render_session_picker(frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
    jinn_domain::feat::session::render::render_session_picker(frame, area, ctx);
}

/// Renders the persona picker overlay (delegates to domain render).
fn render_persona_picker(frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
    jinn_domain::feat::picker::render::render_persona_picker(frame, area, ctx);
}

/// Renders the theme picker overlay (delegates to domain render).
fn render_theme_picker(frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
    jinn_domain::feat::picker::render::render_theme_picker(frame, area, ctx);
}

/// Renders the session lifecycle picker overlay (delegates to domain render).
fn render_session_lifecycle_picker(frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
    jinn_domain::feat::session_lifecycle::render::render_session_lifecycle_picker(frame, area, ctx);
}

/// Renders the workflow picker overlay (delegates to domain render).
fn render_workflow_picker(frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
    jinn_domain::feat::picker::render::render_workflow_picker(frame, area, ctx);
}

/// Renders the arg input popup (delegates to domain render).
pub(super) fn render_arg_input(frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
    jinn_domain::feat::session_lifecycle::render::render_arg_input(frame, area, ctx);
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        reason = "test code, panics are acceptable"
    )]
    use jinn_selection_widget::compute_popup_rect;
    use ratatui::layout::Rect;

    #[rstest::rstest]
    fn larger_terminal_gets_taller_popup() {
        // Given two terminal sizes.
        let small_area = Rect::new(0, 0, 80, 24);
        let large_area = Rect::new(0, 0, 80, 42);

        // When computing popup rects.
        let small_popup = compute_popup_rect(small_area);
        let large_popup = compute_popup_rect(large_area);

        // Then the larger terminal gets a taller popup.
        assert!(large_popup.height > small_popup.height);
    }

    #[rstest::rstest]
    fn small_terminal_uses_75_percent_height() {
        // Given two terminal sizes.
        let small_area = Rect::new(0, 0, 80, 24);
        let large_area = Rect::new(0, 0, 80, 42);

        // When computing popup rects.
        let small_popup = compute_popup_rect(small_area);
        let _large_popup = compute_popup_rect(large_area);

        // Then the small terminal popup uses 75% of height + 4 rows of chrome.
        // floor(24 * 0.75) = 18, min(18 + 4, 24) = 22.
        assert_eq!(small_popup.height, 22);
    }
}
