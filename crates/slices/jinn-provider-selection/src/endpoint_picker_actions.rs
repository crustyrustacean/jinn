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

//! The OpenRouter endpoint picker's pure behavior, over its own cell.
//!
//! Every function here takes `&mut EndpointPickerState` and nothing else: the
//! rows, what a row contributes to the filter, and what confirming one means.
//! Actions that need app state live in `endpoint_picker_routes`.

use jinn_core_types::Endpoint;
use jinn_core_types::model_selection::ModelSelection;
use jinn_picker::{PickerItemHooks, make_items_with_hooks};
use jinn_provider_selection_msg::endpoint::picker_entry::EndpointEntry;
use jinn_provider_selection_msg::endpoint::picker_state::EndpointPickerState;
use jinn_theme::Theme;

/// Whether a model may be routed through a specific OpenRouter endpoint.
///
/// Only a *single* (non-alloy) model can: an alloy is a provider-side
/// abstraction whose own routing is chosen upstream, so pinning an upstream
/// here would either be ignored or would contradict the alloy's purpose.
///
/// This used to be a gate in the kernel's open path, which is why it had to
/// run before the scope push and could not live in the spec's open hook. Now
/// that the slice's `open` action *is* what pushes the scope, the gate lives
/// here and runs at exactly the right moment: decide, then push, or decline.
#[must_use]
pub fn may_route_through_endpoint(model: &ModelSelection) -> bool {
    !matches!(model, ModelSelection::Alloy { .. })
}

/// Opens the picker over the given entries, with a fresh filter and no
/// highlight.
pub fn open(state: &mut EndpointPickerState, entries: Vec<EndpointEntry>, theme: &Theme) {
    state.selection.reset();
    state.theme = theme.clone();
    state.selection.set_items(wrap_entries(entries));
}

/// Replaces the rows, keeping the filter and highlight.
///
/// The provider actor republishes the entry list after a fetch completes, so
/// this is the path that fills the menu a moment after it opens.
pub fn reload(state: &mut EndpointPickerState, entries: Vec<EndpointEntry>) {
    state.selection.set_items(wrap_entries(entries));
}

/// Forgets the rows and the filter, leaving an empty menu.
///
/// Used when a forced refresh starts, so the picker does not keep presenting
/// a stale list as though it were current.
pub fn clear(state: &mut EndpointPickerState) {
    state.selection.reset();
    state.selection.set_items(Vec::new());
}

/// The highlighted endpoint, as the value the session profile pins.
///
/// `None` when nothing is highlighted *or* when the highlight is the
/// auto-route sentinel — both mean "no pinned upstream", so the caller cannot
/// tell them apart and neither needs to.
#[must_use]
pub fn highlighted_endpoint(state: &EndpointPickerState) -> Option<Endpoint> {
    let item = state.selection.selected_item()?;
    entry_to_endpoint(item.entry())
}

/// The label for the status line's `Routing:` field.
///
/// Prefers the active row's fetched provider name, and falls back to the raw
/// routing tag when no fetched row matches. The fallback is what makes a
/// hand-edited pin visible before the list has been fetched, and what keeps a
/// pinned tag legible when the upstream no longer advertises it.
#[must_use]
pub fn active_routing_label(state: &EndpointPickerState) -> Option<String> {
    let tag = state
        .selection
        .items()
        .iter()
        .map(|item| item.entry())
        .find(|entry| entry.is_active)?;

    if tag.provider_name.is_empty() {
        Some(tag.tag.clone())
    } else {
        Some(tag.provider_name.clone())
    }
}

/// The pinned value for one entry: the auto-route sentinel clears the pin.
///
/// Represented as `None` because the session profile stores "no pinned
/// upstream" as an absent endpoint, not as an endpoint with an empty tag.
#[must_use]
pub fn entry_to_endpoint(entry: &EndpointEntry) -> Option<Endpoint> {
    if entry.tag.is_empty() {
        None
    } else {
        Some(Endpoint {
            tag: entry.tag.clone(),
            provider_name: entry.provider_name.clone(),
        })
    }
}

/// The search text for one endpoint row: provider name, then tag.
///
/// The auto-route sentinel has no tag, so it searches on its name alone.
#[must_use]
pub fn search_text(entry: &EndpointEntry) -> String {
    if entry.tag.is_empty() {
        entry.provider_name.clone()
    } else {
        format!("{} {}", entry.provider_name, entry.tag)
    }
}

/// The coarse age of a cached fetch, for the status line.
///
/// `<60s` reads as seconds, `<60m` as minutes, and anything older as hours:
/// the status line is a freshness hint, not a clock, so a second-level figure
/// past an hour would be noise.
#[must_use]
pub fn format_age(fetched_at: jiff::Timestamp) -> String {
    let elapsed = jiff::Timestamp::now() - fetched_at;
    let secs = elapsed.total(jiff::Unit::Second).unwrap_or(0.0).max(0.0) as u64;
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 60 * 60 {
        format!("{}m ago", secs / 60)
    } else {
        format!("{}h ago", secs / 3600)
    }
}

/// Wraps entries for the selection widget, wiring the row renderer, the
/// preview, the preview's cache key, and the text the filter matches against.
///
/// Without these hooks the widget draws an empty label on every row and shows
/// no preview at all, so they are part of the picker's definition rather than
/// decoration.
fn wrap_entries(entries: Vec<EndpointEntry>) -> Vec<jinn_picker::PickerEntry<EndpointEntry>> {
    make_items_with_hooks(
        entries,
        PickerItemHooks::new()
            .row(crate::endpoint_picker_render::endpoint_row)
            .search(search_text)
            .preview(crate::endpoint_picker_render::endpoint_preview)
            // Metadata is static per entry, so cache the preview by tag: the
            // pane must not re-render every frame the highlight sits still.
            .preview_key(|entry: &EndpointEntry| {
                (!entry.tag.is_empty()).then(|| jinn_picker::PreviewKey(entry.tag.clone()))
            }),
    )
}

#[cfg(test)]
mod routing_label_tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        reason = "test module, panics are acceptable"
    )]

    use jinn_theme::default_theme;

    use super::*;

    /// A picker showing `entries`.
    fn picker_with(entries: Vec<EndpointEntry>) -> EndpointPickerState {
        let mut state = EndpointPickerState::default();
        state.selection.set_items(wrap_entries(entries));
        state
    }

    /// A real upstream row, active or not.
    fn row(tag: &str, provider_name: &str, is_active: bool) -> EndpointEntry {
        EndpointEntry {
            tag: tag.to_owned(),
            provider_name: provider_name.to_owned(),
            uptime_30m: None,
            prompt_price: None,
            completion_price: None,
            quantization: None,
            max_completion_tokens: None,
            is_active,
            theme: default_theme(),
        }
    }

    #[rstest::rstest]
    fn the_label_prefers_the_active_rows_provider_name() {
        // Given a picker whose active row came from a completed fetch.
        let state = picker_with(vec![
            EndpointEntry::auto_route(false, default_theme()),
            row("anthropic", "Anthropic", true),
        ]);

        // When reading the routing label.
        let label = active_routing_label(&state);

        // Then the human-readable name is shown.
        assert_eq!(label.as_deref(), Some("Anthropic"));
    }

    #[rstest::rstest]
    fn the_label_falls_back_to_the_tag_when_the_row_has_no_name() {
        // Given a picker whose active row carries no fetched provider name.
        let state = picker_with(vec![
            EndpointEntry::auto_route(false, default_theme()),
            row("anthropic", "", true),
        ]);

        // When reading the routing label.
        let label = active_routing_label(&state);

        // Then the raw routing tag is shown instead of an empty field.
        assert_eq!(label.as_deref(), Some("anthropic"));
    }

    #[rstest::rstest]
    fn the_label_names_the_sentinel_when_the_model_auto_routes() {
        // Given a picker on auto-route, where the sentinel is the active row.
        let state = picker_with(vec![EndpointEntry::auto_route(true, default_theme())]);

        // When reading the routing label.
        let label = active_routing_label(&state);

        // Then the sentinel's own name is reported, not the empty tag.
        assert_eq!(label.as_deref(), Some("Default"));
    }

    #[rstest::rstest]
    fn the_label_is_absent_when_no_row_is_active() {
        // Given a picker whose rows are all inactive.
        let state = picker_with(vec![row("anthropic", "Anthropic", false)]);

        // When reading the routing label.
        let label = active_routing_label(&state);

        // Then there is nothing to name — the status line shows auto-route.
        assert!(label.is_none());
    }
}
