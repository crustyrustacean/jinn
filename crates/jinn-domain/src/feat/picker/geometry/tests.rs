//! Tests for [`super`].

use ratatui::layout::Rect;

use crate::PickerKind;
use crate::common::app_state::AppState;
use crate::feat::picker::geometry::{
    PICKER_VIEWPORT_FALLBACK, measure_active_picker_results_height,
};
/// The domain's registry (spec-backed kinds measure from their specs).
use crate::feat::ui::picker_states::PickerExt;

/// The domain's registry (spec-backed kinds measure from their specs).
fn registry() -> jinn_picker::PickerRegistry {
    crate::feat::picker::test_registry::test_registry()
}

/// Frame area used by the standard popup-fit scenarios below. Large enough
/// that the popup hits its max-height cap.
const LARGE_FRAME: Rect = Rect::new(0, 0, 120, 50);

#[rstest::rstest]
#[test]
fn measure_returns_fallback_when_no_picker_active() {
    // Given default app state with no picker open.
    let state = AppState::default_with_scope_focus();

    // When measuring the active picker viewport.
    let height = measure_active_picker_results_height(&state, LARGE_FRAME, &registry());

    // Then the fallback height is returned.
    assert_eq!(height, PICKER_VIEWPORT_FALLBACK);
}

#[rstest::rstest]
#[test]
fn measure_writes_into_state_field() {
    // Given a default app state.
    let mut state = AppState::default_with_scope_focus();

    // When writing a measured viewport directly.
    state.frontend.set_picker_results_viewport(7);

    // Then the field reflects the written value.
    assert_eq!(state.frontend.picker_results_viewport(), 7);
}

fn state_with_picker(kind: PickerKind) -> AppState {
    use jinn_slices::FocusScope;
    let state = AppState::default_with_scope_focus();
    state.frontend.scope_push(FocusScope::Picker { kind });
    state
}

#[rstest::rstest]
#[test]
fn measure_provider_picker_reserves_two_footer_rows() {
    // Given a Provider picker active (renders refresh + mode footers).
    let state = state_with_picker(PickerKind::Provider);

    // When measuring at LARGE_FRAME.
    let height = measure_active_picker_results_height(&state, LARGE_FRAME, &registry());

    // Then the height is inner minus chrome (2) minus two footers.
    // At LARGE_FRAME the popup inner is 39 rows; 39 - 2 - 2 = 35.
    assert_eq!(height, 35);
}

#[rstest::rstest]
#[test]
fn measure_theme_picker_reserves_two_bottom_rows() {
    // Given a Persona picker active (status + keybind footer via its spec).
    let state = state_with_picker(PickerKind::Theme);

    // When measuring at LARGE_FRAME.
    let height = measure_active_picker_results_height(&state, LARGE_FRAME, &registry());

    // Then the height is inner minus chrome (2) minus the spec's two bottom
    // rows. At LARGE_FRAME the popup inner is 39 rows; 39 - 2 - 2 = 35.
    assert_eq!(height, 35);
}

#[rstest::rstest]
#[test]
fn measure_tiny_frame_never_returns_zero() {
    // Given a Persona picker active on a tiny frame.
    let state = state_with_picker(PickerKind::Theme);

    // When measuring at a 1x1 frame.
    let tiny = Rect::new(0, 0, 1, 1);
    let height = measure_active_picker_results_height(&state, tiny, &registry());

    // Then height is at least 1.
    assert!(height >= 1);
}
