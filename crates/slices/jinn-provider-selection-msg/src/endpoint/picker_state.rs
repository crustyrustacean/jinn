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

//! The OpenRouter endpoint picker's per-open state.
//!
//! One home for everything the menu shows. The kernel used to hold this
//! selection; now the only copy lives here, so the render pass and the key
//! actions cannot end up reading one store while a different one is written.

use jinn_slices::SlotKey;
use jinn_theme::Theme;

use crate::endpoint::picker_entry::EndpointEntry;
use crate::endpoint::picker_scope::endpoint_picker_scope;

/// How many rows the result pane measured, set by the render pass.
///
/// Paging needs a real row count: `SelectionState`'s `max_visible` argument
/// decides whether the scroll window follows the cursor, so a constant would
/// let the highlight walk off-screen.
pub const RESULTS_VIEWPORT_FALLBACK: usize = 20;

/// The endpoint picker's per-open state.
#[derive(Debug)]
pub struct EndpointPickerState {
    /// The rows, the filter text, and the highlight.
    pub selection: jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<EndpointEntry>>,
    /// How many result rows fit on screen, measured by the render pass.
    pub results_viewport: usize,
    /// The theme the rows were built with, so a repaint recolors them
    /// consistently instead of stranding them on a stale palette.
    pub theme: Theme,
}

impl Default for EndpointPickerState {
    fn default() -> Self {
        Self {
            selection: jinn_selection_widget::SelectionState::new(),
            // Matches the kernel's pre-measurement fallback, so the very
            // first keypress before any render behaves as it always has.
            results_viewport: RESULTS_VIEWPORT_FALLBACK,
            theme: jinn_theme::default_theme(),
        }
    }
}

/// The cell key the endpoint picker registers at activation.
#[must_use]
pub fn endpoint_picker_slot() -> SlotKey {
    SlotKey::builtin("provider-selection", "endpoint-picker")
}

/// The picker's scope, re-exported for callers that already import the state.
#[must_use]
pub fn picker_scope() -> jinn_slices::SliceScopeId {
    endpoint_picker_scope()
}
