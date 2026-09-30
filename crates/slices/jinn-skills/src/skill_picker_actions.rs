//! The skill picker's actions — pure functions over the picker's own cell.
//!
//! Each action is a snapshot-transform, act, drop sequence: read what it
//! needs, mutate the cell, release the guard. Nothing here borrows across an
//! await point, and nothing reaches into the kernel — the session-facing
//! effects are returned as values the route layer turns into messages.

use std::collections::HashSet;

use jinn_core_types::NameFilter;
use jinn_skills_msg::{Skill, SkillPickerState};

use crate::skill_picker_reload::build_skill_entries;

/// Opening the picker: fresh filter and selection, snapshot the session's
/// skill filter for the ESC revert, and load entries from the discovered
/// skills.
pub fn open(
    state: &mut SkillPickerState,
    discovered: &[Skill],
    skill_filter: Option<&NameFilter>,
    theme: &jinn_theme::Theme,
) {
    state.reset();
    state.snapshot = Some(skill_filter.cloned());
    state.theme = theme.clone();
    state
        .selection
        .set_items(build_skill_entries(discovered, skill_filter, theme));
}

/// Enter: commit the toggled set as the session's disabled skills, and return
/// it for the caller to apply. The snapshot is cleared without restoring — the
/// commit is authoritative.
///
/// Returns the disabled set to persist.
#[must_use]
pub fn confirm(state: &mut SkillPickerState) -> HashSet<String> {
    let disabled: HashSet<String> = state
        .selection
        .items()
        .iter()
        .filter(|item| !item.entry().enabled)
        .map(|item| item.entry().name.clone())
        .collect();
    state.snapshot = None;
    disabled
}

/// ESC (the revert path — never the confirm path): restore the snapshotted
/// filter.
///
/// Returns the filter to restore, or `None` when the picker was never opened
/// (so the caller leaves the session's filter alone). The whole filter
/// comes back, mode included — handing back only its withheld names would
/// demote an allow-mode session on the way out of a picker nothing changed
/// in.
#[must_use]
pub fn cancel_filter(state: &mut SkillPickerState) -> Option<Option<NameFilter>> {
    state.snapshot.take()
}

/// Toggle the highlighted skill's enabled flag. The picker stays open.
pub fn toggle_highlighted(state: &mut SkillPickerState) {
    state
        .selection
        .with_selected_mut(|item| item.entry_mut().enabled = !item.entry().enabled);
}

/// The highlighted skill's name, if any row is selected.
#[must_use]
pub fn highlighted_name(state: &SkillPickerState) -> Option<String> {
    state
        .selection
        .selected_item()
        .map(|item| item.entry().name.clone())
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        reason = "test module, panics are acceptable"
    )]
    use super::*;

    fn skill(name: &str) -> Skill {
        Skill {
            name: name.to_owned(),
            description: format!("{name} description"),
            body: String::new(),
            file_path: std::path::PathBuf::from(format!("/tmp/{name}/SKILL.md")),
            base_dir: std::path::PathBuf::from(format!("/tmp/{name}")),
            source: jinn_skills_msg::SkillSource::Global,
        }
    }

    fn opened(disabled: &[&str]) -> SkillPickerState {
        let mut state = SkillPickerState::default();
        open(
            &mut state,
            &[skill("alpha"), skill("beta")],
            Some(&NameFilter::deny(disabled.iter().map(|s| (*s).to_owned()))),
            &jinn_theme::default_theme(),
        );
        state
    }

    /// A picker over two skills, holding the given filter.
    fn opened_with(filter: &NameFilter) -> SkillPickerState {
        let mut state = SkillPickerState::default();
        open(
            &mut state,
            &[skill("alpha"), skill("beta")],
            Some(filter),
            &jinn_theme::default_theme(),
        );
        state
    }

    #[rstest::rstest]
    fn open_snapshots_the_filter_for_revert() {
        // Given a session withholding one skill.
        let filter = NameFilter::deny(["beta".to_owned()]);

        // When opening the picker.
        let state = opened_with(&filter);

        // Then the snapshot holds that filter, so ESC can restore it.
        assert_eq!(state.snapshot, Some(Some(filter)));
    }

    #[rstest::rstest]
    fn open_seeds_a_row_from_an_allow_filter_that_omits_it() {
        // Given an allow filter naming only one of the two skills.
        let filter = NameFilter {
            mode: jinn_core_types::FilterMode::Allow,
            names: ["alpha".to_owned()].into_iter().collect(),
        };

        // When opening the picker over both.
        let state = opened_with(&filter);

        // Then the omitted skill's row opens already off, so committing
        // without touching it withholds the same skill the filter did.
        let beta = state
            .selection
            .items()
            .iter()
            .find(|item| item.entry().name == "beta")
            .expect("beta has a row");
        assert!(!beta.entry().enabled);
    }

    #[rstest::rstest]
    fn cancel_restores_the_snapshot_verbatim() {
        // Given a picker opened with an allow filter, then toggled.
        let filter = NameFilter {
            mode: jinn_core_types::FilterMode::Allow,
            names: ["alpha".to_owned()].into_iter().collect(),
        };
        let mut state = opened_with(&filter);
        toggle_highlighted(&mut state);

        // When cancelling.
        let restored = cancel_filter(&mut state);

        // Then the filter comes back with its mode intact — handing back only
        // its withheld names would have demoted it to a blocklist.
        assert_eq!(restored, Some(Some(filter)));
        assert!(state.snapshot.is_none());
    }

    #[rstest::rstest]
    fn toggle_flips_the_highlighted_row_and_keeps_the_picker_open() {
        // Given a freshly opened picker with the first row highlighted.
        let mut state = opened(&[]);

        // When toggling.
        toggle_highlighted(&mut state);

        // Then that row is now disabled and the picker holds its state (no
        // close signal — toggling never leaves the picker).
        let first = state
            .selection
            .items()
            .first()
            .expect("opened picker has rows");
        assert!(!first.entry().enabled);
        assert_eq!(highlighted_name(&state).as_deref(), Some("alpha"));
    }

    #[rstest::rstest]
    fn confirm_returns_the_toggled_disabled_set_and_clears_the_snapshot() {
        // Given a picker whose first row was toggled off.
        let mut state = opened(&[]);
        toggle_highlighted(&mut state);

        // When confirming.
        let disabled = confirm(&mut state);

        // Then the toggled-off skill is the disabled set, and the snapshot is
        // gone so a later ESC cannot undo the commit.
        assert_eq!(disabled, ["alpha".to_owned()].into_iter().collect());
        assert!(state.snapshot.is_none());
    }

    #[rstest::rstest]
    fn cancel_on_a_never_opened_picker_restores_nothing() {
        // Given a picker with no snapshot.
        let mut state = SkillPickerState::default();

        // When cancelling.
        let restored = cancel_filter(&mut state);

        // Then nothing is restored, so the session's filter is left alone.
        assert!(restored.is_none());
    }
}
