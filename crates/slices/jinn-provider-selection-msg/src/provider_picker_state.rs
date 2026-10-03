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

//! The model picker's identity and per-open state.
//!
//! Distinct from [`ProviderCell`], which holds the *discovered data* every
//! reader shares (the model cache, alloy mode, the endpoint fetch stamp).
//! This holds what only the open menu mutates: the list, the filter, the
//! highlight, and the measured viewport.

use crate::ProviderPickerEntry;
use jinn_picker::PickerEntry;
use jinn_slices::SliceScopeId;
use jinn_slices::SlotKey;

/// The row count to assume before the first render pass.
///
/// A *fallback*, not a default: the render pass overwrites it on the first
/// frame. It matches the row count the picker assumes before any
/// measurement lands, so paging behaves identically in the frame or two
/// before one arrives.
pub const RESULTS_VIEWPORT_FALLBACK: usize = 20;

/// The model picker's dynamic scope.
#[must_use]
pub fn provider_picker_scope() -> SliceScopeId {
    SliceScopeId::new("provider-selection", "picker")
}

/// The model picker's per-open state.
#[derive(Debug)]
pub struct ProviderPickerState {
    /// The rows, the filter, and the highlight.
    pub selection: jinn_selection_widget::SelectionState<PickerEntry<ProviderPickerEntry>>,
    /// How many result rows fit on screen, measured by the render pass.
    pub results_viewport: usize,
}

impl Default for ProviderPickerState {
    fn default() -> Self {
        Self {
            selection: jinn_selection_widget::SelectionState::new(),
            results_viewport: RESULTS_VIEWPORT_FALLBACK,
        }
    }
}

/// The cell key the model picker registers at activation.
#[must_use]
pub fn provider_picker_slot() -> SlotKey {
    SlotKey::builtin("provider-selection", "picker")
}
