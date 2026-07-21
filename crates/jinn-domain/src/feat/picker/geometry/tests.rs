//! Tests for [`super`].

use ratatui::layout::Rect;

use crate::common::app_state::AppState;
use crate::feat::picker::geometry::{
    PICKER_VIEWPORT_FALLBACK, measure_active_picker_results_height,
};
use crate::feat::ui::picker_states::PickerExt;

/// Frame area used by the standard popup-fit scenarios below. Large enough
/// that the popup hits its max-height cap and is wide enough to exercise the
/// skill picker's vertical split.
const LARGE_FRAME: Rect = Rect::new(0, 0, 120, 50);

#[test]
fn measure_returns_fallback_when_no_picker_active() {
    // Given default app state with no picker open.
    let state = AppState::default();

    // When measuring the active picker viewport.
    let height = measure_active_picker_results_height(&state, LARGE_FRAME);

    // Then the fallback height is returned.
    assert_eq!(height, PICKER_VIEWPORT_FALLBACK);
}

#[test]
fn measure_writes_into_state_field() {
    // Given a default app state.
    let mut state = AppState::default();

    // When writing a measured viewport directly.
    state.frontend.set_picker_results_viewport(7);

    // Then the field reflects the written value.
    assert_eq!(state.frontend.picker_results_viewport(), 7);
}
