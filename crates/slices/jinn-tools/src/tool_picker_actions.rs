// Copyright (C) 2026 Jayson Lennon
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! The tool picker's row building and lifecycle operations.
//!
//! Pure functions over the picker's own state. They read what they need, mutate
//! the cell, and return what the caller needs to write to the session — they
//! never reach session state themselves, which is what keeps the picker inside
//! its slice.

use std::collections::HashSet;

use jinn_picker::picker_style::{dim_style, split_match_indices};
use jinn_picker::{PickerItemHooks, make_items_with_hooks};
use jinn_selection_widget::highlight::highlight_text_with_bg;
use jinn_theme::Theme;
use jinn_tools_msg::{ToolEntry, ToolPickerState};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

/// Renders one picker row: the ✓/✗ enablement marker, the tool name, and an
/// em-dash description.
///
/// Filter matches highlight within the name and the description separately —
/// byte offsets from the search text `"{name} {description}"` split at the
/// separator.
#[must_use]
pub fn tool_row(entry: &ToolEntry, ctx: &jinn_picker::RowCtx<'_>) -> Line<'static> {
    let style = if ctx.is_selected {
        Style::default()
            .fg(entry.theme.primary_text)
            .bg(entry.theme.picker_selected_bg)
    } else {
        Style::default()
    };

    let (marker, marker_color) = if entry.enabled {
        ("\u{2713} ", entry.theme.focus_accent) // ✓
    } else {
        ("\u{2717} ", entry.theme.error_text) // ✗
    };
    let marker_span = Span::styled(marker.to_owned(), Style::default().fg(marker_color));

    // Match ranges are byte offsets into "{name} {description}"; the space
    // separator sits at byte offset `name_len`.
    let (name_indices, desc_indices) = split_match_indices(ctx.match_ranges, entry.name.len());

    let name_spans = highlight_text_with_bg(
        &entry.name,
        style,
        &name_indices,
        entry.theme.picker_highlight_bg,
    );
    let desc_style = dim_style(ctx.is_selected, &entry.theme);
    let sep_span = Span::styled(" \u{2014} ".to_owned(), desc_style);
    let desc_spans = highlight_text_with_bg(
        &entry.description,
        desc_style,
        &desc_indices,
        entry.theme.picker_highlight_bg,
    );

    let mut spans = vec![marker_span];
    spans.extend(name_spans);
    spans.push(sep_span);
    spans.extend(desc_spans);
    Line::from(spans)
}

/// The definitions the picker offers, already narrowed to what the session's
/// provider can actually run and seeded from the live disabled set.
#[derive(Debug, Clone)]
pub struct ToolRow {
    /// The tool's registered name.
    pub name: String,
    /// The tool's registered description.
    pub description: String,
}

/// Opens the picker: fresh filter and highlight over `definitions`, and a
/// snapshot of the session's disabled set so escape can put it back.
///
/// The disabled set is read *only* here, to seed the rows and to take the
/// snapshot. It is written back only on confirm.
pub fn open(
    state: &mut ToolPickerState,
    definitions: &[ToolRow],
    disabled: &HashSet<String>,
    theme: &Theme,
) {
    state.reset();
    state.snapshot = Some(disabled.clone());

    let entries: Vec<ToolEntry> = definitions
        .iter()
        .map(|def| ToolEntry {
            name: def.name.clone(),
            description: def.description.clone(),
            enabled: !disabled.contains(&def.name),
            theme: theme.clone(),
        })
        .collect();
    state.selection.set_items(wrap_entries(entries));
}

/// Wraps entries for the selection widget, wiring the row renderer and the text
/// the filter matches against.
///
/// Without these hooks the widget draws an empty label on every row, so they are
/// part of the picker's definition rather than decoration.
fn wrap_entries(entries: Vec<ToolEntry>) -> Vec<jinn_picker::PickerEntry<ToolEntry>> {
    make_items_with_hooks(
        entries,
        PickerItemHooks::new()
            .row(tool_row)
            .search(|entry: &ToolEntry| format!("{} {}", entry.name, entry.description)),
    )
}

/// Commits the toggled set: the names of every row the user turned off.
///
/// The snapshot is cleared without restoring — the commit is authoritative, so
/// a later escape must not undo it. Nothing here writes to the session; the
/// caller does, and only here.
#[must_use]
pub fn confirm(state: &mut ToolPickerState) -> HashSet<String> {
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

/// Escape (the revert path — never the confirm path): restore the snapshotted
/// disabled set.
///
/// Returns the set to restore, or `None` when the picker was never opened (or
/// was already committed), so the caller leaves the session's set alone.
#[must_use]
pub fn cancel(state: &mut ToolPickerState) -> Option<HashSet<String>> {
    state.snapshot.take()
}

/// Tab: flip the highlighted tool's enabled flag, then advance to the next row
/// by the measured viewport (checklist style) so a run of adjacent tools can be
/// toggled without moving the cursor first.
///
/// A no-op when no row is highlighted — an empty tool context must not panic.
pub fn toggle_highlighted(state: &mut ToolPickerState) {
    if state.selection.selected_item().is_none() {
        return;
    }
    state
        .selection
        .with_selected_mut(|item| item.entry_mut().enabled = !item.entry().enabled);
    let viewport = state.results_viewport;
    state.selection.move_down(viewport);
}

/// The highlighted tool's name, if any row is selected.
#[must_use]
pub fn highlighted_name(state: &ToolPickerState) -> Option<String> {
    state
        .selection
        .selected_item()
        .map(|item| item.entry().name.clone())
}

/// How many tools are currently enabled, and how many the picker offers.
#[must_use]
pub fn enabled_count(state: &ToolPickerState) -> (usize, usize) {
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

    /// One row per name, ordered as the session's registry returns them.
    fn rows(names: &[&str]) -> Vec<ToolRow> {
        names
            .iter()
            .map(|name| ToolRow {
                name: (*name).to_owned(),
                description: format!("{name} description"),
            })
            .collect()
    }

    /// A picker opened over `names` with `disabled` already turned off.
    fn opened(names: &[&str], disabled: &[&str]) -> ToolPickerState {
        let mut state = ToolPickerState::default();
        let set: HashSet<String> = disabled.iter().map(|s| (*s).to_owned()).collect();
        open(&mut state, &rows(names), &set, &jinn_theme::default_theme());
        state
    }

    #[rstest::rstest]
    fn open_seeds_each_row_from_the_disabled_set() {
        // Given a session with one tool already disabled.
        // When opening the picker.
        let state = opened(&["bash", "read"], &["read"]);

        // Then the disabled tool's row renders disabled and the other enabled.
        let enabled_of = |name: &str| {
            state
                .selection
                .items()
                .iter()
                .find(|item| item.entry().name == name)
                .map(|item| item.entry().enabled)
        };
        assert_eq!(enabled_of("read"), Some(false));
        assert_eq!(enabled_of("bash"), Some(true));
    }

    #[rstest::rstest]
    fn open_snapshots_the_disabled_set_for_the_escape_revert() {
        // Given a session with one tool disabled.
        // When opening the picker.
        let state = opened(&["bash", "read"], &["read"]);

        // Then the snapshot holds that set, so escape can restore it.
        assert_eq!(
            state.snapshot,
            Some(["read".to_owned()].into_iter().collect::<HashSet<String>>())
        );
    }

    #[rstest::rstest]
    fn toggle_flips_the_highlighted_row_and_advances_the_highlight() {
        // Given a freshly opened picker with the first row highlighted.
        let mut state = opened(&["bash", "read"], &[]);

        // When toggling.
        toggle_highlighted(&mut state);

        // Then that row is disabled and the highlight moved to the next row.
        let first = state
            .selection
            .items()
            .first()
            .expect("opened picker has rows");
        assert!(!first.entry().enabled);
        assert_eq!(highlighted_name(&state).as_deref(), Some("read"));
    }

    #[rstest::rstest]
    fn toggling_an_empty_picker_is_a_no_op() {
        // Given a picker opened over no tools at all.
        let mut state = opened(&[], &[]);

        // When toggling.
        toggle_highlighted(&mut state);

        // Then nothing is selected and nothing changed.
        assert!(state.selection.selected_item().is_none());
    }

    #[rstest::rstest]
    fn toggling_does_not_touch_the_snapshotted_disabled_set() {
        // Given an opened picker whose snapshot is the pre-open set.
        let mut state = opened(&["bash", "read"], &[]);

        // When toggling a row off.
        toggle_highlighted(&mut state);

        // Then the snapshot is untouched — the disabled set is only ever
        // written on confirm, so a toggle can never pre-commit anything.
        assert_eq!(state.snapshot, Some(HashSet::new()));
    }

    #[rstest::rstest]
    fn confirm_returns_the_toggled_off_names_and_clears_the_snapshot() {
        // Given an opened picker whose first row was toggled off.
        let mut state = opened(&["bash", "read"], &[]);
        toggle_highlighted(&mut state);

        // When confirming.
        let disabled = confirm(&mut state);

        // Then the toggled-off tool is the disabled set, and the snapshot is
        // gone so a later escape cannot undo the commit.
        assert_eq!(disabled, ["bash".to_owned()].into_iter().collect());
        assert!(state.snapshot.is_none());
    }

    #[rstest::rstest]
    fn cancel_restores_the_snapshotted_set() {
        // Given a picker opened with a tool disabled, then toggled.
        let mut state = opened(&["bash", "read"], &["read"]);
        toggle_highlighted(&mut state);

        // When cancelling.
        let restored = cancel(&mut state);

        // Then the pre-open set comes back and the snapshot is consumed.
        assert_eq!(
            restored,
            Some(["read".to_owned()].into_iter().collect::<HashSet<String>>())
        );
        assert!(state.snapshot.is_none());
    }

    #[rstest::rstest]
    fn cancel_on_a_never_opened_picker_restores_nothing() {
        // Given a picker with no snapshot.
        let mut state = ToolPickerState::default();

        // When cancelling.
        let restored = cancel(&mut state);

        // Then nothing is restored, so the session's set is left alone.
        assert!(restored.is_none());
    }

    #[rstest::rstest]
    fn enabled_count_reports_the_toggle_state_live() {
        // Given a picker with one of two tools turned off.
        let state = opened(&["bash", "read"], &["read"]);

        // When counting the enabled rows.
        let (enabled, total) = enabled_count(&state);

        // Then one of two is enabled.
        assert_eq!((enabled, total), (1, 2));
    }

    #[rstest::rstest]
    fn an_enabled_row_shows_a_check_and_its_description() {
        // Given an enabled tool entry rendered unselected.
        let entry = ToolEntry {
            name: "bash".to_owned(),
            description: "Run shell".to_owned(),
            enabled: true,
            theme: jinn_theme::default_theme(),
        };

        // When rendering its row.
        let line = tool_row(&entry, &jinn_picker::RowCtx::flat(false, &[]));

        // Then the check marker, the name, and the em-dash description appear.
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains('\u{2713}'), "got {text:?}");
        assert!(text.contains("bash") && text.contains("Run shell"));
    }

    #[rstest::rstest]
    fn a_disabled_row_shows_a_cross_instead_of_a_check() {
        // Given a disabled tool entry.
        let entry = ToolEntry {
            name: "bash".to_owned(),
            description: "Run shell".to_owned(),
            enabled: false,
            theme: jinn_theme::default_theme(),
        };

        // When rendering its row.
        let line = tool_row(&entry, &jinn_picker::RowCtx::flat(false, &[]));

        // Then the cross marker replaces the check.
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains('\u{2717}'), "got {text:?}");
        assert!(!text.contains('\u{2713}'));
    }
}
