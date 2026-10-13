//! The stream-rule picker's actions — pure functions over the picker's own
//! cell.
//!
//! Each action is a snapshot-transform, act, drop sequence: read what it
//! needs, mutate the cell, release the guard. Nothing here borrows across an
//! await point, and nothing reaches into the kernel — the persisted effects
//! happen in the route layer, where the confirm has the dispatch context.

use jinn_preferences_config::schemas::StreamRuleConfig;
use jinn_stream_rule_picker_msg::{StreamRulePickerState, build_stream_rule_entries};

/// Opening the picker: fresh filter, selection, and status; capture the
/// theme; and load the rows from the configured rules with each entry's
/// pending state seeded at the persisted value.
pub fn open(
    state: &mut StreamRulePickerState,
    rules: &[StreamRuleConfig],
    theme: &jinn_theme::Theme,
) {
    state.reset();
    state.theme = theme.clone();
    state
        .selection
        .set_items(build_stream_rule_entries(rules, theme));
}

/// The highlighted rule's name, if any row is selected.
#[must_use]
pub fn highlighted_name(state: &StreamRulePickerState) -> Option<String> {
    state
        .selection
        .selected_item()
        .map(|item| item.entry().name.clone())
}

/// The highlighted rule's pending disabled state, if any row is selected.
#[must_use]
pub fn highlighted_disabled(state: &StreamRulePickerState) -> Option<bool> {
    state
        .selection
        .selected_item()
        .map(|item| item.entry().disabled)
}

/// Toggles the highlighted rule's pending disabled state. The picker stays
/// open and nothing is persisted — the write belongs to the confirm.
pub fn toggle_highlighted(state: &mut StreamRulePickerState) {
    state
        .selection
        .with_selected_mut(|item| item.entry_mut().disabled = !item.entry().disabled);
}

/// Collects the pending set as `(name, disabled)` pairs, in display order.
#[must_use]
pub fn pending_set(state: &StreamRulePickerState) -> Vec<(String, bool)> {
    state
        .selection
        .items()
        .iter()
        .map(|item| (item.entry().name.clone(), item.entry().disabled))
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        reason = "test module, panics are acceptable"
    )]
    use super::*;
    use jinn_preferences_config::schemas::StreamRuleConfig;
    use jinn_theme::default_theme;

    fn rule(name: &str) -> StreamRuleConfig {
        StreamRuleConfig {
            name: name.to_owned(),
            description: format!("the {name} rule"),
            conditions: vec!["TODO".to_owned()],
            body: "Finish it.".to_owned(),
            ..Default::default()
        }
    }

    /// The pending disabled flag of the named row, or `None` when absent.
    fn pending_of(state: &StreamRulePickerState, name: &str) -> Option<bool> {
        state
            .selection
            .items()
            .iter()
            .find(|item| item.entry().name == name)
            .map(|item| item.entry().disabled)
    }

    #[rstest::rstest]
    fn open_seeds_the_rows_from_the_configured_rules() {
        // Given a default picker state and two configured rules.
        let mut state = StreamRulePickerState::default();

        // When opening the picker over them.
        open(&mut state, &[rule("alpha"), rule("beta")], &default_theme());

        // Then both are shown in configuration order.
        let names: Vec<String> = state
            .selection
            .items()
            .iter()
            .map(|item| item.entry().name.clone())
            .collect();
        assert_eq!(names, ["alpha", "beta"]);
    }

    #[rstest::rstest]
    fn open_seeds_each_row_at_its_persisted_state() {
        // Given a configuration where the second rule is persisted disabled.
        let mut beta = rule("beta");
        beta.disabled = true;

        // When opening the picker over it.
        let mut state = StreamRulePickerState::default();
        open(&mut state, &[rule("alpha"), beta], &default_theme());

        // Then the pending set starts at the persisted values — a fresh open
        // reflects what the file holds, not the last session's edits.
        assert_eq!(pending_of(&state, "alpha"), Some(false));
        assert_eq!(pending_of(&state, "beta"), Some(true));
    }

    #[rstest::rstest]
    fn open_resets_the_filter_the_highlight_and_the_status() {
        // Given a picker opened once, filtered, moved off the top, and left
        // with a status line.
        let mut state = StreamRulePickerState::default();
        open(&mut state, &[rule("alpha"), rule("beta")], &default_theme());
        state.selection.insert_char('a');
        state.selection.move_down(state.results_viewport);
        state.status = Some("disabled 'beta'".to_owned());

        // When reopening it.
        open(&mut state, &[rule("alpha"), rule("beta")], &default_theme());

        // Then the filter is gone, the highlight is back on the first row,
        // and the stale status is cleared.
        assert!(state.selection.filter().is_empty());
        assert_eq!(highlighted_name(&state).as_deref(), Some("alpha"));
        assert_eq!(state.status, None);
    }

    #[rstest::rstest]
    fn toggle_flips_only_the_highlighted_row_and_persists_nothing() {
        // Given an open picker with the highlight on the first rule.
        let mut state = StreamRulePickerState::default();
        open(&mut state, &[rule("alpha"), rule("beta")], &default_theme());

        // When toggling.
        toggle_highlighted(&mut state);

        // Then only the highlighted rule's pending state flipped — the
        // sibling is untouched, and nothing has left the picker's cell.
        assert_eq!(pending_of(&state, "alpha"), Some(true));
        assert_eq!(pending_of(&state, "beta"), Some(false));
    }

    #[rstest::rstest]
    fn toggling_twice_returns_the_row_to_its_original_state() {
        // Given an open picker whose highlighted rule was toggled once.
        let mut state = StreamRulePickerState::default();
        open(&mut state, &[rule("alpha")], &default_theme());
        toggle_highlighted(&mut state);

        // When toggling again.
        toggle_highlighted(&mut state);

        // Then the pending state is back where it started.
        assert_eq!(pending_of(&state, "alpha"), Some(false));
    }

    #[rstest::rstest]
    fn highlighted_name_and_state_are_none_without_rows() {
        // Given a picker that was never opened.
        let state = StreamRulePickerState::default();

        // When asking for the highlighted name and state.
        // Then there are none.
        assert_eq!(highlighted_name(&state), None);
        assert_eq!(highlighted_disabled(&state), None);
    }

    #[rstest::rstest]
    fn pending_set_reports_every_row_in_display_order() {
        // Given an open picker where the first rule was toggled off.
        let mut state = StreamRulePickerState::default();
        open(&mut state, &[rule("alpha"), rule("beta")], &default_theme());
        toggle_highlighted(&mut state);

        // When collecting the pending set.
        let pending = pending_set(&state);

        // Then every row appears, each with its pending flag.
        assert_eq!(
            pending,
            vec![("alpha".to_owned(), true), ("beta".to_owned(), false)]
        );
    }
}
