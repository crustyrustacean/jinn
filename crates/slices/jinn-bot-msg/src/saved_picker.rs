//! The saved-bots picker's wire contracts: its cell slot, dynamic
//! scope, and state.
//!
//! The picker browses `[[bot.entry]]` — the bots the user has
//! saved from the properties popup — and confirms one to create it on the
//! active session. Like every picker, the payload lives in the `-msg` crate
//! and the slice owns the rows, rendering, and the creation itself.
//!
//! The payload is [`SavedBotSummary`], a flat view of one entry, not
//! the config type itself: the config crate depends on *this* crate for its
//! behavior/trigger vocabulary, so a cell holding the config entry would
//! close the loop into a cycle. The summary also keeps the cell renderable
//! without a `jinn.toml` read on every frame.

use jinn_slices::SlotKey;

/// The `bot/saved-picker` slot: the saved-bots picker's state.
#[must_use]
pub fn bot_saved_picker_slot() -> SlotKey {
    SlotKey::builtin("bot", "saved-picker")
}

/// The saved-bots picker's dynamic scope.
#[must_use]
pub fn bot_saved_picker_scope() -> jinn_slices::slice_scope::SliceScopeId {
    jinn_slices::slice_scope::SliceScopeId::new("bot", "saved-picker")
}

/// Result rows assumed before the render pass has measured the real popup.
///
/// Matches the kernel's pre-measurement fallback, so the first keypress
/// after opening the picker pages the same way it always has.
pub const RESULTS_VIEWPORT_FALLBACK: usize = 20;

/// One row in the saved-bots picker.
///
/// The name is the whole entry: it is what the row shows, and it is the
/// key the created bot is looked up by. The run configuration the
/// entry carries is not summarized here — the picker creates a bot,
/// it does not describe one, and a row that previewed a configuration the
/// user cannot change from this popup is a second thing to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedBotSummary {
    /// The entry's name — also the title the created bot is given.
    pub name: String,
}

/// State for the saved-bots picker.
#[derive(Debug)]
pub struct BotSavedPickerState {
    /// The rows, the filter text, and the highlight.
    pub selection: jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<SavedBotSummary>>,
    /// How many result rows fit on screen, measured by the render pass.
    pub results_viewport: usize,
}

impl Default for BotSavedPickerState {
    fn default() -> Self {
        Self {
            selection: jinn_selection_widget::SelectionState::default(),
            results_viewport: RESULTS_VIEWPORT_FALLBACK,
        }
    }
}
