//! Attendant properties popup — the edit state for its three controls.
//!
//! The popup edits an attendant's trigger, activation, and seed template.
//! The editable text and cursor live in [`LineInput`] (shared with other
//! popup inputs); the toggles cycle through their enum variants in place.

use jinn_slices::LineInput;
use jinn_slices::SlotKey;

/// The `attendant/properties` slot: the properties popup's single edit state.
#[must_use]
pub fn attendant_properties_slot() -> SlotKey {
    SlotKey::builtin("attendant", "properties")
}

/// State for the attendant properties popup.
///
/// Opened from the sessions section with `P`; all three fields target the
/// highlighted attendant session.
#[derive(Debug, Clone, Default)]
pub struct AttendantPropertiesState {
    /// The session the popup is editing. `None` while the popup is closed.
    pub session_id: Option<jinn_core_types::SessionId>,
    /// The editable seed-template text + cursor.
    pub seed_template: LineInput,
}
