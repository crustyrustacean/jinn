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

//! The task-list picker's identity and per-open state.
//!
//! The task list is a *tree* menu: phases are roots, tasks are children, and
//! the widget does the expanding. So the state is a `TreePickerState` rather
//! than a flat `SelectionState` — the two have different navigation, and
//! picking the wrong one silently loses indent and collapse behavior.

use crate::task_list_entry::TaskListTreeEntry;
use jinn_slices::SliceScopeId;
use jinn_slices::SlotKey;

/// How many rows the result pane measured, set by the render pass.
pub const RESULTS_VIEWPORT_FALLBACK: usize = 20;

/// The task-list picker's dynamic scope (input-capturing: the filter is text).
#[must_use]
pub fn task_list_picker_scope() -> SliceScopeId {
    SliceScopeId::new("tools", "task-list")
}

/// The task-list picker's per-open state.
#[derive(Debug)]
pub struct TaskListPickerState {
    /// The tree: phases as roots, tasks as children, plus the filter text and
    /// the expanded/collapsed view the widget derives from them.
    pub tree: jinn_selection_widget::TreePickerState<jinn_picker::PickerEntry<TaskListTreeEntry>>,
    /// How many result rows fit on screen, measured by the render pass.
    pub results_viewport: usize,
}

impl Default for TaskListPickerState {
    fn default() -> Self {
        Self {
            tree: jinn_selection_widget::TreePickerState::new(),
            // Matches the kernel's pre-measurement fallback, so the very
            // first keypress before any render behaves as it always has.
            results_viewport: RESULTS_VIEWPORT_FALLBACK,
        }
    }
}

/// The cell key the task-list picker registers at activation.
#[must_use]
pub fn task_list_picker_slot() -> SlotKey {
    SlotKey::builtin("tools", "task-list-picker")
}
