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

//! The reasoning picker's state, as a slice cell.
//!
//! The picker is owned by the provider-selection slice: this payload is its
//! only home. Nothing in the kernel holds a second copy, so the menu cannot be
//! left showing one store while a different one is written.

use jinn_slices::SlotKey;
use jinn_theme::Theme;

use super::ReasoningEffortEntry;

/// How many rows the result pane measured, set by the render pass.
///
/// Paging needs a real row count: `SelectionState`'s `max_visible` argument
/// decides whether the scroll window follows the cursor, so a constant would
/// let the highlight walk off-screen.
pub const RESULTS_VIEWPORT_FALLBACK: usize = 20;

/// The reasoning picker's per-open state.
#[derive(Debug)]
pub struct ReasoningPickerState {
    /// The rows, the filter text, and the highlight.
    pub selection:
        jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<ReasoningEffortEntry>>,
    /// How many result rows fit on screen, measured by the render pass.
    pub results_viewport: usize,
    /// The theme the rows were built with, so a repaint recolors them
    /// consistently instead of stranding them on a stale palette.
    pub theme: Theme,
    /// The effort name the status line reports, seeded when the picker opens.
    ///
    /// The render pass cannot read app state, so the picker records what the
    /// status should say rather than resolving the session's effort again.
    pub active_name: Option<String>,
}

impl Default for ReasoningPickerState {
    fn default() -> Self {
        Self {
            selection: jinn_selection_widget::SelectionState::new(),
            // Matches the kernel's pre-measurement fallback, so the very
            // first keypress before any render behaves as it always has.
            results_viewport: RESULTS_VIEWPORT_FALLBACK,
            theme: jinn_theme::default_theme(),
            active_name: None,
        }
    }
}

/// The cell key the reasoning picker registers at activation.
///
/// Namespaced per picker so the provider-selection slice's other pickers can
/// claim their own slots beside it.
#[must_use]
pub fn reasoning_picker_slot() -> SlotKey {
    SlotKey::builtin("provider-selection", "reasoning-picker")
}
