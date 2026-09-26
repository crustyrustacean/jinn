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

//! The model picker's pure behavior, over its own cell.
//!
//! Alloy *mode* lives in [`ProviderCell`] rather than here, because the
//! provider actor reads it while building rows — mode is shared data, the
//! list is menu state.

use jinn_core_types::model_selection::{AlloyStrategy, ModelSelection};
use jinn_picker::PickerEntry;
use jinn_provider_selection_msg::ProviderPickerEntry;
use jinn_provider_selection_msg::ProviderPickerState;

/// The picker type the actions operate on.
type List = jinn_selection_widget::SelectionState<PickerEntry<ProviderPickerEntry>>;

/// Resets to a fresh, empty, unfiltered list.
///
/// Called on open, before the load request: discovery is async, so the popup
/// opens empty and fills in when the actor publishes.
pub fn reset(state: &mut ProviderPickerState) {
    state.selection.reset();
}

/// Installs freshly loaded rows, keeping the filter and highlight.
pub fn load(state: &mut ProviderPickerState, entries: Vec<ProviderPickerEntry>) {
    state.selection.set_items(wrap_entries(entries));
}

/// The provider id under the highlight, if the row is available.
///
/// `None` when nothing is highlighted *or* the highlight is unavailable: an
/// unavailable model cannot be selected, so confirm treats both the same.
#[must_use]
pub fn available_highlight(state: &ProviderPickerState) -> Option<String> {
    let item = state.selection.selected_item()?;
    if !item.entry().is_available {
        return None;
    }
    Some(item.entry().provider_id.clone())
}

/// Flips the highlight's alloy check, then floats checked rows to the top.
///
/// No cursor movement: a checked row changing rank under the cursor would make
/// TAB feel like it moved the highlight. Alloy mode only.
pub fn toggle_highlighted(state: &mut ProviderPickerState) {
    state
        .selection
        .with_selected_mut(|item| item.entry_mut().selected = !item.entry().selected);
    resort(state);
}

/// Pre-checks the session's current models, or clears every check.
///
/// Called when alloy mode is entered or left. Both branches end with a resort
/// so the checked rows float back to the top.
pub fn set_alloy_membership(state: &mut ProviderPickerState, model_selection: &ModelSelection) {
    let mut items = state.selection.items().to_vec();
    for item in &mut items {
        if matches!(model_selection, ModelSelection::Alloy { .. }) {
            jinn_provider_selection_msg::pre_check_active_models(
                std::slice::from_mut(item.entry_mut()),
                model_selection,
            );
        } else {
            item.entry_mut().selected = false;
        }
    }
    state.selection.set_items(items);
    resort(state);
}

/// How many rows are checked for the alloy.
#[must_use]
pub fn selected_count(state: &ProviderPickerState) -> usize {
    state
        .selection
        .items()
        .iter()
        .filter(|item| item.entry().selected)
        .count()
}

/// Resolves what a confirm should commit.
///
/// Single mode: the highlight becomes `Single`. Alloy mode: the checked set
/// union the highlight, deduped — one model resolves back to `Single`, two or
/// more to an `Alloy` on a round-robin.
#[must_use]
pub fn resolve_selection(
    state: &ProviderPickerState,
    alloy_mode: bool,
    highlighted: String,
) -> ModelSelection {
    if !alloy_mode {
        return ModelSelection::Single(highlighted);
    }
    let mut models: Vec<String> = state
        .selection
        .items()
        .iter()
        .filter(|item| item.entry().selected && item.entry().is_available)
        .map(|item| item.entry().provider_id.clone())
        .collect();

    // Force-include the highlight: ENTER adds it before committing, so a user
    // who tabs nothing and confirms gets the row under the cursor.
    if !models.contains(&highlighted) {
        models.push(highlighted);
    }

    if models.len() <= 1 {
        ModelSelection::Single(models.into_iter().next().unwrap_or_default())
    } else {
        ModelSelection::Alloy {
            models,
            strategy: AlloyStrategy::RoundRobin { index: 0 },
        }
    }
}

/// Re-sorts so checked entries float to the top, stable within each group.
fn resort(state: &mut ProviderPickerState) {
    let mut items: Vec<PickerEntry<ProviderPickerEntry>> = state.selection.items().to_vec();
    items.sort_by_key(|item| !item.entry().selected);
    state.selection.set_items(items);
}

/// The text the filter matches a row by.
#[must_use]
pub fn search_text(entry: &ProviderPickerEntry) -> String {
    format!("{} {}", entry.model, entry.provider_name)
}

/// Wraps entries for the selection widget.
#[must_use]
pub fn wrap_entries(entries: Vec<ProviderPickerEntry>) -> Vec<PickerEntry<ProviderPickerEntry>> {
    jinn_picker::make_items_with_hooks(
        entries,
        jinn_picker::PickerItemHooks::new()
            .row(provider_row)
            .search(search_text),
    )
}

/// Renders one row: the ✓ marker (alloy members), a status prefix
/// (✗ unavailable / → alias / * remote), the model name, and the provider name
/// in parens. Filter matches highlight the model and provider portions
/// separately — byte offsets from the search text `"{model} {provider_name}"`
/// split at the separator.
fn provider_row(
    entry: &ProviderPickerEntry,
    ctx: &jinn_picker::RowCtx<'_>,
) -> ratatui::text::Line<'static> {
    use jinn_picker::picker_style::{selected_style, split_match_indices};
    use ratatui::style::Style;
    use ratatui::text::Span;

    let selection_marker = if entry.selected {
        Span::styled(
            "\u{2713} ".to_owned(),
            Style::default().fg(entry.theme.picker_active_marker),
        )
    } else {
        Span::styled("  ".to_owned(), Style::default())
    };

    let status_prefix = if !entry.is_available {
        "\u{2717} "
    } else if entry.is_alias {
        "\u{2192} "
    } else if entry.is_remote {
        "* "
    } else {
        "  "
    };

    let label_style = if entry.is_available {
        selected_style(ctx.is_selected, &entry.theme)
    } else {
        Style::default().fg(entry.theme.muted_text)
    };
    let highlight_bg = entry.theme.picker_highlight_bg;

    let (model_indices, provider_indices) =
        split_match_indices(ctx.match_ranges, entry.model.len());

    let mut spans = Vec::new();
    if entry.is_alias {
        spans.push(Span::styled(
            format!("{}{} \u{2192} ", status_prefix, entry.name),
            label_style,
        ));
    } else {
        spans.push(Span::styled(status_prefix.to_owned(), label_style));
    }
    spans.extend(jinn_selection_widget::highlight_text_with_bg(
        &entry.model,
        label_style,
        &model_indices,
        highlight_bg,
    ));
    spans.push(Span::styled(" (".to_owned(), label_style));
    spans.extend(jinn_selection_widget::highlight_text_with_bg(
        &entry.provider_name,
        label_style,
        &provider_indices,
        highlight_bg,
    ));
    spans.push(Span::styled(")".to_owned(), label_style));

    ratatui::text::Line::from(
        std::iter::once(selection_marker)
            .chain(spans)
            .collect::<Vec<_>>(),
    )
}

/// The list the navigation keys operate on.
#[must_use]
pub fn list(state: &ProviderPickerState) -> &List {
    &state.selection
}

/// The list the navigation and filter keys operate on, mutably.
#[must_use]
pub fn list_mut(state: &mut ProviderPickerState) -> &mut List {
    &mut state.selection
}
