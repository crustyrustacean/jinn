//! The two conversions between a live attendant session and its saved
//! `[[attendant.entry]]` form.
//!
//! Kept in one module because they are a matched pair: whatever save drops
//! as "not configured" is exactly what create inherits back from the parent
//! session, and a field added to one without the other is a pin dropped or
//! an inheritance that silently does not happen.

use jinn_attendant_msg::AttendantTrigger;
use jinn_core_types::{ChatEntry, ChatEntryId};
use jinn_preferences_config::schemas::AttendantEntryConfig;
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
    let entry = AttendantEntryConfig::from_parts(
        name,
        session.attendant_activation(),
        session.attendant_trigger(),
        session.seed_template().to_owned(),
        &profile.model,
        &profile.persona_name,
        &profile.disabled_tools,
        &profile.disabled_skills,
        profile.reasoning_effort,
        profile.endpoint.as_ref(),
        pinned_entries(session),
    );
    entry
}

/// The session's pinned entries, in history order.
///
/// Order is the whole point: a saved attendant's context is a *sequence*
/// of pinned instructions, and replaying them in a different order builds a
/// different attendant. Tool loops stay contiguous because the editor's
/// pin is chunk-scoped — pinning one member pins the whole run, so the
/// members are already adjacent in a saved session's history.
#[must_use]
pub fn pinned_entries(session: &ChatSessionState) -> Vec<ChatEntry> {
    session
        .history()
        .iter()
        .filter(|entry| entry.pin_position().is_some())
        .cloned()
        .collect()
}

/// Appends the entry's pins to a fresh attendant, in order, with fresh
/// entry ids.
///
/// The ids are regenerated because a new session's history is its own: a
/// replayed id would be a different entry sharing a name with one from the
/// session that was saved, and every id-keyed view state in the UI (the
/// selected entry, the streaming target) would resolve to the wrong one.
///
/// The pin position rides on the entry itself — including the kind-level
/// pin a tool result carries inside its kind — so the restored pin is the
/// saved pin without a chunk re-pin. Re-pinning through the editor would be
/// the alternative, and it is deliberately not done: the editor's pin is
/// chunk-scoped, so a re-pin would rewrite the positions of *every* member
/// of whatever chunk the new entry joined, rather than the one position the
/// user chose. The loop's members arrive contiguous and in the saved
/// order, so the history this builds is the history that was pinned.
pub fn restore_pins(session: &mut ChatSessionState, pins: &[ChatEntry]) {
    for saved in pins {
        let mut entry = saved.clone();
        entry.id = ChatEntryId::new();
        session.push_entry(entry);
    }
}
