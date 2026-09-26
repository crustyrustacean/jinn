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

//! The reasoning picker's row building and lifecycle operations.
//!
//! Pure functions over the picker's own state. They read what they need,
//! mutate the picker, and return what the caller needs to turn into
//! messages — they never reach session state themselves, which is what keeps
//! the picker inside its slice.

use jinn_picker::make_items_with_hooks;
use jinn_provider_selection_msg::ReasoningEffort;
use jinn_provider_selection_msg::ReasoningEffortEntry;
use jinn_provider_selection_msg::reasoning::reasoning_row;
use jinn_theme::Theme;

/// All seven effort variants in declaration order.
///
/// Kept in sync with the `ReasoningEffort` enum. Serves as the single source
/// of truth for the picker's row order.
const ALL_EFFORTS: [ReasoningEffort; 7] = [
    ReasoningEffort::Max,
    ReasoningEffort::Xhigh,
    ReasoningEffort::High,
    ReasoningEffort::Medium,
    ReasoningEffort::Low,
    ReasoningEffort::Minimal,
    ReasoningEffort::None,
];

/// Human-readable description for each effort variant.
///
/// Display-only; the wire value is [`ReasoningEffort::as_str`].
const fn effort_description(effort: ReasoningEffort) -> &'static str {
    match effort {
        ReasoningEffort::Max => "Maximum effort",
        ReasoningEffort::Xhigh => "Extra-high effort",
        ReasoningEffort::High => "High effort",
        ReasoningEffort::Medium => "Medium effort",
        ReasoningEffort::Low => "Low effort",
        ReasoningEffort::Minimal => "Minimal effort",
        ReasoningEffort::None => "Skip reasoning",
    }
}

/// Builds one entry per effort variant, marking the session's own resolved
/// effort active.
#[must_use]
pub fn build_effort_entries(
    active: Option<ReasoningEffort>,
    theme: &Theme,
) -> Vec<ReasoningEffortEntry> {
    ALL_EFFORTS
        .iter()
        .map(|&effort| ReasoningEffortEntry {
            effort,
            name: effort.as_str().to_owned(),
            description: effort_description(effort).to_owned(),
            is_active: active == Some(effort),
            theme: theme.clone(),
        })
        .collect()
}

/// Wraps entries for the selection widget, wiring the row renderer and the
/// text the filter matches against.
///
/// Without these hooks the widget draws an empty label on every row, so they
/// are part of the picker's definition rather than decoration.
fn wrap_entries(
    entries: Vec<ReasoningEffortEntry>,
) -> Vec<jinn_picker::PickerEntry<ReasoningEffortEntry>> {
    make_items_with_hooks(
        entries,
        jinn_picker::PickerItemHooks::new()
            .row(reasoning_row)
            .search(|entry: &ReasoningEffortEntry| format!("{} {}", entry.name, entry.description)),
    )
}

/// Opens the picker: fresh filter and highlight over the seven effort
/// variants, seeded with the session's own resolved effort.
///
/// `active` is the session's resolved effort, not the global default: effort
/// is session-owned, so opening on the global would mark the wrong row for
/// every session that has since made its own choice.
pub fn open(
    state: &mut jinn_provider_selection_msg::ReasoningPickerState,
    active: Option<ReasoningEffort>,
    theme: &Theme,
) {
    state.selection.reset();
    state.theme = theme.clone();
    state.active_name = active.map(|effort| effort.as_str().to_owned());
    state
        .selection
        .set_items(wrap_entries(build_effort_entries(active, theme)));
}

/// The effort the highlight currently selects, if any.
#[must_use]
pub fn highlighted_effort(
    state: &jinn_provider_selection_msg::ReasoningPickerState,
) -> Option<ReasoningEffort> {
    state
        .selection
        .selected_item()
        .map(|item| item.entry().effort)
}

/// The status line: the session's active effort, e.g. `Active: high`, or
/// `Active: none` when the session has no effort of its own.
#[must_use]
pub fn reasoning_status(
    state: &jinn_provider_selection_msg::ReasoningPickerState,
    theme: &Theme,
) -> ratatui::text::Line<'static> {
    let active = state.active_name.as_deref().unwrap_or("none");
    ratatui::text::Line::from(vec![
        ratatui::text::Span::styled(
            "Active: ".to_owned(),
            ratatui::style::Style::default().fg(theme.muted_text),
        ),
        ratatui::text::Span::styled(
            active.to_owned(),
            ratatui::style::Style::default().fg(theme.primary_text),
        ),
    ])
}
