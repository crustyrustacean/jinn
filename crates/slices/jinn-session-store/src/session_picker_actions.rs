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

//! The session picker's pure behavior, over its own cell.
//!
//! The rows are *not* built here: reading the session history is a genuine
//! SQLite round trip, so the actor loads them and hands them back. These
//! functions only reshape what it produced.

use jinn_picker::{PickerItemHooks, make_items_with_hooks};
use jinn_session_store_msg::SessionPickerState;
use jinn_session_store_msg::SessionTreeEntry;

/// Resets to a fresh, empty, unfiltered tree.
///
/// Called on open, before the load request: the history read is async, so the
/// picker shows an empty tree until the actor fills it rather than blocking the
/// keypress that opened it.
pub fn reset(state: &mut SessionPickerState) {
    state.tree = jinn_selection_widget::TreePickerState::new();
}

/// Forgets every row, leaving an empty tree.
pub fn clear(state: &mut SessionPickerState) {
    reset(state);
}

/// Installs freshly loaded rows, keeping the filter.
///
/// Keeping the filter is deliberate: a refresh (a new session finishing, say)
/// must not silently discard what the user had typed to narrow a long history.
pub fn load(state: &mut SessionPickerState, entries: Vec<SessionTreeEntry>) {
    state.tree.set_items(wrap_entries(entries));
}

/// Replaces the rows wholesale, from a fresh filter and highlight.
pub fn open_with_entries(state: &mut SessionPickerState, entries: Vec<SessionTreeEntry>) {
    reset(state);
    load(state, entries);
}

/// The session id under the highlight, if there is one.
#[must_use]
pub fn highlighted_session(state: &SessionPickerState) -> Option<jinn_core_types::SessionId> {
    state
        .tree
        .selected_item()
        .map(|item| item.entry().session_id.clone())
}

/// The text the filter matches a row by.
#[must_use]
pub fn search_text(entry: &SessionTreeEntry) -> String {
    entry.title.clone()
}

/// Wraps entries for the tree widget.
///
/// The row renderer comes from the canonical `SessionTreeEntry` module rather
/// than a copy: the kernel's own session-entry writer and this picker supply
/// the same function, and two copies of a column-padding routine would drift.
fn wrap_entries(entries: Vec<SessionTreeEntry>) -> Vec<jinn_picker::PickerEntry<SessionTreeEntry>> {
    make_items_with_hooks(
        entries,
        PickerItemHooks::new()
            .row(jinn_session_store_msg::session_row)
            .search(search_text),
    )
}
