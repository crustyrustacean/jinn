//! The two conversions between a live attendant session and its saved
//! `[[attendant.entry]]` form.
//!
//! Kept in one module because they are a matched pair: whatever save drops
//! as "not configured" is exactly what create inherits back from the parent
//! session, and a field added to one without the other is a pin dropped or
//! an inheritance that silently does not happen.

use jinn_attendant_msg::AttendantTrigger;
use jinn_core_types::PinPosition;
use jinn_preferences_config::schemas::{AttendantEntryConfig, AttendantPinConfig};
use jinn_session_state::ChatSessionState;

/// What the save path has to decide before it writes anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveIntent {
    /// No entry shares this name: the first `<c-s>` saves.
    New,
    /// An entry shares this name: the first `<c-s>` arms, the second
    /// overwrites.
    Overwrite,
}

/// The trigger's label as it appears in a picker row.
///
/// A `String` rather than the enum because the row is display text; the enum
/// stays the value the entry stores.
#[must_use]
pub fn trigger_label(trigger: AttendantTrigger) -> String {
    match trigger {
        AttendantTrigger::Manual => "manual".to_owned(),
        AttendantTrigger::ParentCompleted => "parent-completed".to_owned(),
    }
}

/// Whether an entry named `name` already exists in the live document.
#[must_use]
pub fn save_intent(config: &jinn_config::ConfigLayer, name: &str) -> SaveIntent {
    let existing = config
        .get_list::<AttendantEntryConfig>()
        .unwrap_or_default();
    if existing.iter().any(|entry| entry.name == name) {
        SaveIntent::Overwrite
    } else {
        SaveIntent::New
    }
}

/// Builds the entry that describes `session` right now.
///
/// Pending popup edits are *not* applied first: the pending values are the
/// user's scratch work, and a save that recorded them would persist an
/// edit the user never committed. The entry describes the session as it
/// stands; `<enter>` then applies the pending edits, and a second save
/// captures them.
#[must_use]
pub fn entry_for_session(name: String, session: &ChatSessionState) -> AttendantEntryConfig {
    let profile = session.profile();
    AttendantEntryConfig::from_parts(
        name,
        session.attendant_behavior(),
        session.attendant_trigger(),
        session.attendant_is_prepping(),
        session.seed_template().to_owned(),
        &profile.model,
        &profile.persona_name,
        &profile.disabled_tools,
        &profile.disabled_skills,
        profile.reasoning_effort,
        profile.endpoint.as_ref(),
        pinned_entries(session),
    )
}

/// The session's standing instructions, in history order.
///
/// Order is the whole point: a saved attendant's context is a *sequence* of
/// pinned instructions, and replaying them in a different order builds a
/// different attendant.
///
/// Only user and assistant entries are carried, because those are the kinds
/// `entries_to_messages` turns into a message. Pinning a tool result in a
/// live session is still fully supported — it simply does not cross into the
/// spawn schema, where re-injecting a snapshot of a file as it was would
/// assert stale contents as current fact.
#[must_use]
pub fn pinned_entries(session: &ChatSessionState) -> Vec<AttendantPinConfig> {
    session
        .history()
        .iter()
        .filter(|entry| entry.pin_position().is_some())
        .filter_map(AttendantPinConfig::from_entry)
        .collect()
}

/// Appends the entry's instructions to a fresh attendant, in order.
///
/// Each is restored as a *relative* pin. The config stores no pin position —
/// a restored attendant is a new session, so where an instruction sat in the
/// session it was saved from says nothing about where it should sit here —
/// and the session store owns the position from this point on: the user
/// re-pins in the TUI and the store persists it. Re-saving an attendant
/// therefore returns every pin to `Relative`, which is the cost of not
/// storing it.
///
/// `Relative` rather than no pin at all is what makes the instruction
/// survive a run with a `reset` behavior: reset force-excludes everything
/// that is not pinned, so an unpinned instruction would be dropped before
/// the model ever saw it.
///
/// Ids are fresh for the same reason they always were — a new session's
/// history is its own, and a replayed id would make every id-keyed view
/// state resolve to an entry from the session that was saved. Every other
/// field comes from the constructor, which is where a token count belongs:
/// `fill_missing_token_counts` fills only entries that carry none, so a
/// persisted count would never be recomputed and would stay stale forever.
pub fn restore_pins(session: &mut ChatSessionState, pins: &[AttendantPinConfig]) {
    for pin in pins {
        let mut entry = pin.to_entry();
        entry.pin_position = Some(PinPosition::Relative);
        session.push_entry(entry);
    }
}
