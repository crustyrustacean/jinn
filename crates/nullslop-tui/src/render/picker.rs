//! Picker overlay rendering — dispatches to domain-specific picker renderers.

use nullslop_domain::AppState;
use nullslop_domain::PickerKind;
use ratatui::Frame;
use ratatui::layout::Rect;

/// Renders the active picker overlay, dispatching on [`PickerKind`].
pub(super) fn render_picker(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    match state.frontend.scope_stack.picker_kind().copied() {
        Some(PickerKind::Provider) => render_provider_picker(frame, area, state),
        Some(PickerKind::ContextAssembly) => {
            render_context_strategy_picker(frame, area, state);
        }
        Some(PickerKind::Keymap) => render_keymap_picker(frame, area, state),
        Some(PickerKind::Session) => render_session_picker(frame, area, state),
        Some(PickerKind::Persona) => render_persona_picker(frame, area, state),
        Some(PickerKind::Theme) => render_theme_picker(frame, area, state),
        Some(PickerKind::SessionFork) => render_session_fork_picker(frame, area, state),
        None => {}
    }
}

/// Renders the provider picker overlay (delegates to slice).
fn render_provider_picker(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    nullslop_domain::feat::provider::render::render_provider_picker(frame, area, state);
}

/// Renders the context strategy picker overlay (delegates to slice).
fn render_context_strategy_picker(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    nullslop_domain::feat::picker::render::render_context_strategy_picker(frame, area, state);
}

/// Renders the keymap picker overlay (delegates to slice).
fn render_keymap_picker(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    nullslop_domain::feat::picker::render::render_keymap_picker(frame, area, state);
}

/// Renders the session picker overlay (delegates to slice).
fn render_session_picker(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    nullslop_domain::feat::session::render::render_session_picker(frame, area, state);
}

/// Renders the persona picker overlay (delegates to domain render).
fn render_persona_picker(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    nullslop_domain::feat::picker::render::render_persona_picker(frame, area, state);
}

/// Renders the theme picker overlay (delegates to domain render).
fn render_theme_picker(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    nullslop_domain::feat::picker::render::render_theme_picker(frame, area, state);
}

/// Renders the session fork picker overlay (delegates to domain render).
fn render_session_fork_picker(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    nullslop_domain::feat::session::render::render_session_fork_picker(frame, area, state);
}

#[cfg(test)]
mod tests {
    use nullslop_selection_widget::compute_popup_rect;
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
