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
// along with this program.  If not, not, see <https://www.gnu.org/licenses/>.

//! Signals from the [`IntentHandler`] for the outer platform layer.
//!
//! The [`IntentHandler`] sets these flags during processing. The platform layer
//! (`TuiApp` or headless runner) reads them after each `handle()` call and
//! performs the corresponding platform-specific action.
//!
//! All flags are cleared at the start of each `handle()` call so they are
//! always fresh.

/// Flags set by the [`IntentHandler`] for the outer platform layer to act on.
///
/// These represent requests that cannot be fulfilled by the [`IntentHandler`]
/// itself because they require platform-specific machinery (TUI widgets,
/// external editor, split manager, etc.).
#[derive(Debug)]
pub struct TuiSignals {
    /// The which-key popup should be toggled (shown ↔ hidden).
    pub toggle_whichkey: bool,

    /// An external editor should be launched for the chat input.
    pub edit_requested: bool,
}

impl Default for TuiSignals {
    fn default() -> Self {
        Self::new()
    }
}

impl TuiSignals {
    /// Create with all signals cleared.
    #[must_use]
    pub fn new() -> Self {
        Self {
            toggle_whichkey: false,
            edit_requested: false,
        }
    }

    /// Clear all signals. Called at the start of each `handle()` call.
    pub fn clear(&mut self) {
        self.toggle_whichkey = false;
        self.edit_requested = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn clear_resets_all_signals() {
        // Given a TuiSignals with some flags set.
        let mut signals = TuiSignals::new();
        signals.toggle_whichkey = true;
        signals.edit_requested = true;
        signals.toggle_whichkey = true;

        // When clearing.
        signals.clear();

        // Then all flags are false.
        assert!(!signals.toggle_whichkey);
        assert!(!signals.edit_requested);
    }
}
