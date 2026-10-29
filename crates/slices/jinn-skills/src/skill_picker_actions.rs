//! The skill picker's actions — pure functions over the picker's own cell.
//!
//! Each action is a snapshot-transform, act, drop sequence: read what it
//! needs, mutate the cell, release the guard. Nothing here borrows across an
//! await point, and nothing reaches into the kernel — the session-facing
//! effects are returned as values the route layer turns into messages.

use std::collections::HashSet;

use jinn_core_types::NameFilter;
use jinn_llm_support::token_estimator::TiktokenCounter;
use jinn_skills_msg::{Skill, SkillPickerState};

use crate::format::{loaded_skill_tokens, skills_list_tokens};
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
    // The full discovered skills ride along (paths included) so the list-cost
    // figure is counted from exactly the block a prompt would carry.
    state.discovered = discovered.to_vec();
    recompute_tokens(state);
}

/// Recomputes both cost terms of the picker's pending figure.
///
/// The **list term** counts the `<available_skills>` block of the enabled rows
/// — the block a confirmed filter would ship — with an empty loaded set: the
/// picker's pending list carries no `loaded="true"` marks, because loaded-ness
/// is a fact about history, not a picker toggle. The **loaded term** counts
/// the captured pinned bodies; toggling never touches it, since disabling a
/// skill removes only its list line while the body stays pinned.
pub fn recompute_tokens(state: &mut SkillPickerState) {
    let enabled: HashSet<&str> = state
        .selection
        .items()
        .iter()
        .filter(|item| item.entry().enabled)
        .map(|item| item.entry().name.as_str())
        .collect();
    let counter = TiktokenCounter::o200k_base();
    let enabled_skills: Vec<Skill> = state
        .discovered
        .iter()
        .filter(|skill| enabled.contains(skill.name.as_str()))
        .cloned()
        .collect();
    let loaded: std::collections::HashSet<String> = std::collections::HashSet::new();
    state.list_tokens = skills_list_tokens(&enabled_skills, &loaded, &counter);
    state.loaded_tokens = loaded_skill_tokens(&state.loaded_contents, &counter);
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
///
/// The list-cost term follows the flip; the loaded term does not — disabling
/// a loaded skill removes only its list line, not its pinned body.
pub fn toggle_highlighted(state: &mut SkillPickerState) {
    state
        .selection
        .with_selected_mut(|item| item.entry_mut().enabled = !item.entry().enabled);
    recompute_tokens(state);
}

/// The highlighted skill's name, if any row is selected.
#[must_use]
pub fn highlighted_name(state: &SkillPickerState) -> Option<String> {
    state
        .selection
        .selected_item()
        .map(|item| item.entry().name.clone())
}

/// How many skills are currently enabled, and how many the picker offers.
#[must_use]
pub fn enabled_count(state: &SkillPickerState) -> (usize, usize) {
    let items = state.selection.items();
    let enabled = items.iter().filter(|item| item.entry().enabled).count();
    (enabled, items.len())
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        reason = "test module, panics are acceptable"
    )]
    use super::*;
    use marked_path::{Absolute, MarkedPathBuf};

    fn skill(root: &MarkedPathBuf<Absolute>, name: &str) -> Skill {
        Skill {
            name: name.to_owned(),
            description: format!("{name} description"),
            body: String::new(),
            file_path: root
                .join_all([name, "SKILL.md"])
                .expect("typed parent joins bare names"),
            base_dir: root.join(name).expect("typed parent joins a bare name"),
            source: jinn_skills_msg::SkillSource::Global,
        }
    }

    /// A picker over two skills rooted in `root`, holding the given disabled
    /// list.
    fn opened(root: &MarkedPathBuf<Absolute>, disabled: &[&str]) -> SkillPickerState {
        let mut state = SkillPickerState::default();
        open(
            &mut state,
            &[skill(root, "alpha"), skill(root, "beta")],
            Some(&NameFilter::deny(disabled.iter().map(|s| (*s).to_owned()))),
            &jinn_theme::default_theme(),
        );
        state
    }

    /// A picker over two skills rooted in `root`, holding the given filter.
    fn opened_with(root: &MarkedPathBuf<Absolute>, filter: &NameFilter) -> SkillPickerState {
        let mut state = SkillPickerState::default();
        open(
            &mut state,
            &[skill(root, "alpha"), skill(root, "beta")],
            Some(filter),
            &jinn_theme::default_theme(),
        );
        state
    }

    #[rstest::rstest]
    fn open_snapshots_the_filter_for_revert() {
        // Given a session withholding one skill.
        let dir = jinn_testutil::tempdir();
        let filter = NameFilter::deny(["beta".to_owned()]);

        // When opening the picker.
        let state = opened_with(&dir.path, &filter);

        // Then the snapshot holds that filter, so ESC can restore it.
        assert_eq!(state.snapshot, Some(Some(filter)));
    }

    #[rstest::rstest]
    fn open_seeds_a_row_from_an_allow_filter_that_omits_it() {
        // Given an allow filter naming only one of the two skills.
        let dir = jinn_testutil::tempdir();
        let filter = NameFilter {
            mode: jinn_core_types::FilterMode::Allow,
            names: ["alpha".to_owned()].into_iter().collect(),
        };

        // When opening the picker over both.
        let state = opened_with(&dir.path, &filter);

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
        let dir = jinn_testutil::tempdir();
        let filter = NameFilter {
            mode: jinn_core_types::FilterMode::Allow,
            names: ["alpha".to_owned()].into_iter().collect(),
        };
        let mut state = opened_with(&dir.path, &filter);
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
        let dir = jinn_testutil::tempdir();
        let mut state = opened(&dir.path, &[]);

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
        let dir = jinn_testutil::tempdir();
        let mut state = opened(&dir.path, &[]);
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

    #[rstest::rstest]
    fn list_term_counts_the_enabled_skills_block() {
        // Given a picker over two skills with one withheld.
        let dir = jinn_testutil::tempdir();
        let state = opened(&dir.path, &["beta"]);

        // When reading the list term.
        // Then it equals the counted block of the enabled skill alone.
        let counter = jinn_llm_support::token_estimator::TiktokenCounter::o200k_base();
        let expected = crate::format::skills_list_tokens(
            &[skill(&dir.path, "alpha")],
            &std::collections::HashSet::new(),
            &counter,
        );
        assert_eq!(state.list_tokens, expected);
        assert!(state.list_tokens > 0, "one enabled skill must cost tokens");
    }

    #[rstest::rstest]
    fn loaded_term_counts_the_captured_pinned_contents() {
        // Given a picker whose loaded pairs hold two pinned bodies.
        let dir = jinn_testutil::tempdir();
        let mut state = opened(&dir.path, &[]);
        state.loaded_contents = vec![
            (
                "skill".to_owned(),
                "<skill name=\"a\">body a</skill>".to_owned(),
            ),
            (
                "skill".to_owned(),
                "<skill name=\"b\">body b</skill>".to_owned(),
            ),
        ];

        // When recomputing the terms.
        super::recompute_tokens(&mut state);

        // Then the loaded term prices each pair as name + content, exactly
        // as the assembler's Tool term would.
        let counter = jinn_llm_support::token_estimator::TiktokenCounter::o200k_base();
        let expected = crate::format::loaded_skill_tokens(
            &[
                (
                    "skill".to_owned(),
                    "<skill name=\"a\">body a</skill>".to_owned(),
                ),
                (
                    "skill".to_owned(),
                    "<skill name=\"b\">body b</skill>".to_owned(),
                ),
            ],
            &counter,
        );
        assert_eq!(state.loaded_tokens, expected);
        assert!(state.loaded_tokens > 0);
    }

    #[rstest::rstest]
    fn disabling_a_loaded_skill_keeps_the_loaded_term() {
        // Given a picker with one loaded (pinned) skill and one plain skill.
        let dir = jinn_testutil::tempdir();
        let mut state = opened(&dir.path, &[]);
        state.loaded_contents = vec![(
            "skill".to_owned(),
            "<skill name=\"alpha\">pinned</skill>".to_owned(),
        )];
        super::recompute_tokens(&mut state);
        let list_before = state.list_tokens;
        let loaded_before = state.loaded_tokens;

        // When disabling the loaded skill's row (alpha is highlighted first).
        super::toggle_highlighted(&mut state);

        // Then the loaded term is unchanged: the pinned body keeps costing
        // until it is unloaded.
        assert_eq!(state.loaded_tokens, loaded_before);
        // And the list term now counts beta's block alone.
        let counter = jinn_llm_support::token_estimator::TiktokenCounter::o200k_base();
        let expected = crate::format::skills_list_tokens(
            &[skill(&dir.path, "beta")],
            &std::collections::HashSet::new(),
            &counter,
        );
        assert_eq!(state.list_tokens, expected);
        assert!(
            state.list_tokens < list_before,
            "alpha's line left the list"
        );
    }

    #[rstest::rstest]
    fn loading_a_skill_raises_the_loaded_term() {
        // Given an open picker.
        let dir = jinn_testutil::tempdir();
        let mut state = opened(&dir.path, &[]);
        let loaded_before = state.loaded_tokens;

        // When a load appends the pinned pair and recomputes (the route's
        // load path).
        state.loaded_contents.push((
            "skill".to_owned(),
            "<skill name=\"alpha\">body</skill>".to_owned(),
        ));
        super::recompute_tokens(&mut state);

        // Then the loaded term has grown by that body's cost.
        assert!(state.loaded_tokens > loaded_before);
    }
}
