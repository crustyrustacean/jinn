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

//! The theme picker's state, as a slice cell.
//!
//! The picker is owned by the theme slice: this payload is its only home.
//! Nothing in the kernel holds a second copy, so the menu cannot be left
//! showing one store while a different one is written.

use jinn_slices::SlotKey;
use jinn_theme::Theme;
use jinn_theme::ThemeEntry;

/// How many rows the result pane measured, set by the render pass.
///
/// Paging needs a real row count: `SelectionState`'s `max_visible` argument
/// decides whether the scroll window follows the cursor, so a constant would
/// let the highlight walk off-screen.
pub const RESULTS_VIEWPORT_FALLBACK: usize = 20;

/// The theme picker's per-open state.
#[derive(Debug)]
pub struct ThemePickerState {
    /// The rows, the filter text, and the highlight.
    pub selection: jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<ThemeEntry>>,
    /// How many result rows fit on screen, measured by the render pass.
    pub results_viewport: usize,
    /// The theme the rows were built with, so a repaint recolors them
    /// consistently instead of stranding them on a stale palette.
    pub theme: Theme,
    /// The theme in force when the picker opened, restored on escape.
    ///
    /// The picker live-previews, so moving the highlight changes the app's
    /// theme; without this snapshot the user would have no way back to the
    /// theme they started from. Confirming clears it — the choice is then
    /// authoritative and nothing reverts it.
    pub preview_original: Option<Theme>,
    /// The persisted theme name the status line reports, seeded when the
    /// picker opens.
    ///
    /// The render pass cannot read app state, so the picker records what the
    /// status should say. It is the *persisted* name, not the previewed one:
    /// moving the highlight changes what the app looks like, and the status
    /// must still name what would survive a restart.
    pub persisted_name: Option<String>,
}

impl Default for ThemePickerState {
    fn default() -> Self {
        Self {
            selection: jinn_selection_widget::SelectionState::new(),
            // Matches the kernel's pre-measurement fallback, so the very
            // first keypress before any render behaves as it always has.
            results_viewport: RESULTS_VIEWPORT_FALLBACK,
            theme: jinn_theme::default_theme(),
            preview_original: None,
            persisted_name: None,
        }
    }
}

/// The cell key the theme picker registers at activation.
#[must_use]
pub fn theme_picker_slot() -> SlotKey {
    SlotKey::builtin("theme", "picker")
}
