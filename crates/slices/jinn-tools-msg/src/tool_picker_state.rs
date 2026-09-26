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

//! The tool picker's state, as a slice cell.
//!
//! The picker is owned by the tools slice: this payload is its only home.
//! Nothing in the kernel holds a second copy, so the menu cannot be left
//! showing one store while a different one is written.
//!
//! The cell slot is namespaced per picker. The tools slice will also host the
//! task-list picker, so a generic `tools` slot would collide; the slot is keyed
//! `("tools", "tool-picker")` and the matching scope `("tools", "tool-picker")`
//! to say which of the two it is without reading either implementation.

use std::collections::HashSet;

use jinn_slices::SlotKey;

use crate::tool_entry::ToolEntry;

/// How many rows the result pane measured, set by the render pass.
///
/// Paging needs a real row count: `SelectionState`'s `max_visible` argument
/// decides whether the scroll window follows the cursor, so a constant would
/// let the highlight walk off-screen.
pub const RESULTS_VIEWPORT_FALLBACK: usize = 20;

/// The tool picker's per-open state: what it shows, and the set it restores to
/// on escape.
#[derive(Debug)]
pub struct ToolPickerState {
    /// The rows, the filter text, and the highlight.
    pub selection: jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<ToolEntry>>,
    /// The disabled-tool set captured when the picker opened, or `None` before
    /// the first open.
    ///
    /// Escape restores it; confirm commits the toggled set instead and clears
    /// it, so nothing can revert a choice the user just made. The set is
    /// written to the session **only** on confirm — toggling edits the cell's
    /// rows, never the live profile.
    pub snapshot: Option<HashSet<String>>,
    /// How many result rows fit on screen, measured by the render pass.
    pub results_viewport: usize,
}

impl Default for ToolPickerState {
    fn default() -> Self {
        Self {
            selection: jinn_selection_widget::SelectionState::new(),
            snapshot: None,
            // Matches the kernel's pre-measurement fallback, so the very first
            // keypress before any render behaves as it always has.
            results_viewport: RESULTS_VIEWPORT_FALLBACK,
        }
    }
}

impl ToolPickerState {
    /// Clears the filter and the highlight, ready for a fresh open.
    ///
    /// The snapshot survives: it is the set escape restores to, and reopening
    /// re-captures it from the live profile anyway.
    pub fn reset(&mut self) {
        self.selection.clear_filter();
        self.selection.move_up(usize::MAX);
    }
}

/// The cell key the tool picker registers at activation.
#[must_use]
pub fn tool_picker_slot() -> SlotKey {
    SlotKey::builtin("tools", "tool-picker")
}
