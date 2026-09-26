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

//! The session picker's identity and per-open state.
//!
//! A *tree* menu — subagent sessions nest under their parents — so the state
//! is a `TreePickerState` rather than a flat `SelectionState`. Picking the
//! wrong one silently loses the nesting.

use crate::SessionTreeEntry;
use jinn_slices::SliceScopeId;
use jinn_slices::SlotKey;

/// How many rows the result pane measured, set by the render pass.
pub const RESULTS_VIEWPORT_FALLBACK: usize = 20;

/// The session picker's dynamic scope.
#[must_use]
pub fn session_picker_scope() -> SliceScopeId {
    SliceScopeId::new("session-store", "picker")
}

/// The session picker's per-open state.
#[derive(Debug)]
pub struct SessionPickerState {
    /// The tree: parent sessions as roots, subagent sessions as children,
    /// plus the filter text and the expanded/collapsed view derived from them.
    pub tree: jinn_selection_widget::TreePickerState<jinn_picker::PickerEntry<SessionTreeEntry>>,
    /// How many result rows fit on screen, measured by the render pass.
    pub results_viewport: usize,
}

impl Default for SessionPickerState {
    fn default() -> Self {
        Self {
            tree: jinn_selection_widget::TreePickerState::new(),
            results_viewport: RESULTS_VIEWPORT_FALLBACK,
        }
    }
}

/// The cell key the session picker registers at activation.
#[must_use]
pub fn session_picker_slot() -> SlotKey {
    SlotKey::builtin("session-store", "picker")
}
