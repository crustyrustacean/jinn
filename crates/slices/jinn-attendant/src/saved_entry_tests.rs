//! Tests for the saved-attendant conversions: what a save captures, and
//! what a restore rebuilds.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code"
)]

use std::collections::HashSet;

use jinn_attendant_msg::{AttendantActivation, AttendantTrigger};
use jinn_core_types::{ChatEntry, PinPosition};
use jinn_session_state::ChatSessionState;

use crate::saved_entry;

/// A titled attendant carrying two pins and one unpinned entry between
/// them, in the configuration a save is expected to capture.
fn composed_attendant() -> ChatSessionState {
    let parent = ChatSessionState::new();
    let mut attendant = ChatSessionState::new_attendant(&parent, true);
    attendant.set_title("nightly".to_owned());
    attendant.set_attendant_activation(AttendantActivation::Reset);
    attendant.set_attendant_trigger(AttendantTrigger::ParentCompleted);
    attendant.set_seed_template("review: <prior report>".to_owned());
    attendant.push_entry(ChatEntry {
        pin_position: Some(PinPosition::Top),
        ..ChatEntry::user("first instruction")
    });
    attendant.push_entry(ChatEntry::user("chatter"));
    attendant.push_entry(ChatEntry {
        pin_position: Some(PinPosition::Bottom),
        ..ChatEntry::user("second instruction")
    });
    attendant
}

#[rstest::rstest]
#[test]
fn an_entry_names_the_attendant_it_describes() {
    // Given a titled attendant.
    let session = composed_attendant();

    // When building its entry.
    let entry = saved_entry::entry_for_session("nightly".to_owned(), &session);

    // Then the entry carries that name.
    assert_eq!(entry.name, "nightly");
}

#[rstest::rstest]
#[test]
fn an_entry_records_the_run_configuration() {
    // Given an attendant configured to reset and re-run on the parent.
    let session = composed_attendant();

    // When building its entry.
    let entry = saved_entry::entry_for_session("nightly".to_owned(), &session);

    // Then activation, trigger, and seed template come across as saved, so
    // a created attendant is live from its first run.
    assert_eq!(entry.activation, AttendantActivation::Reset);
    assert_eq!(entry.trigger, AttendantTrigger::ParentCompleted);
    assert_eq!(entry.seed_template, "review: <prior report>");
}

#[rstest::rstest]
#[test]
fn an_entry_records_the_pins_in_history_order() {
    // Given an attendant whose pins straddle an unpinned entry.
    let session = composed_attendant();

    // When building its entry.
    let entry = saved_entry::entry_for_session("nightly".to_owned(), &session);

    // Then both pins are stored in order and the chatter is not stored.
    let texts: Vec<String> = entry.pins.iter().map(ChatEntry::text).collect();
    assert_eq!(texts, vec!["first instruction", "second instruction"]);
    assert_eq!(entry.pins[0].pin_position, Some(PinPosition::Top));
    assert_eq!(entry.pins[1].pin_position, Some(PinPosition::Bottom));
}

#[rstest::rstest]
#[test]
fn an_entry_records_the_disabled_tool_and_skill_sets() {
    // Given an attendant with tools and skills explicitly disabled.
    let mut session = composed_attendant();
    {
        let profile = session.profile_mut();
        profile.disabled_tools = HashSet::from(["write".to_owned(), "bash".to_owned()]);
        profile.disabled_skills = HashSet::from(["dataviz".to_owned()]);
    }

    // When building its entry.
    let entry = saved_entry::entry_for_session("nightly".to_owned(), &session);

    // Then they are stored sorted, so the file does not churn between
    // saves of an unchanged attendant.
    assert_eq!(entry.disabled_tools, vec!["bash", "write"]);
    assert_eq!(entry.disabled_skills, vec!["dataviz"]);
}

#[rstest::rstest]
#[test]
fn an_entry_omits_a_session_that_configured_nothing() {
    // Given a fresh attendant carrying only defaults.
    let parent = ChatSessionState::new();
    let session = ChatSessionState::new_attendant(&parent, true);

    // When building its entry.
    let entry = saved_entry::entry_for_session("bare".to_owned(), &session);

    // Then nothing is configured: the model is the placeholder, the persona
    // the default, and both disablement sets empty — every one of which
    // create inherits back from the parent session.
    assert!(entry.configured_model().is_none());
    assert!(entry.persona_name.is_none());
    assert!(entry.disabled_tools.is_empty());
    assert!(entry.disabled_skills.is_empty());
}

#[rstest::rstest]
#[test]
fn restoring_pins_gives_every_entry_a_fresh_id() {
    // Given a saved entry's pins.
    let session = composed_attendant();
    let entry = saved_entry::entry_for_session("nightly".to_owned(), &session);
    let saved_ids: Vec<String> = entry.pins.iter().map(|e| e.id.to_string()).collect();

    // When restoring them into a fresh attendant.
    let parent = ChatSessionState::new();
    let mut fresh = ChatSessionState::new_attendant(&parent, true);
    saved_entry::restore_pins(&mut fresh, &entry.pins);

    // Then the entries are there, in order, with ids that are not the ones
    // the saved session used — a replayed id would be a different entry
    // sharing a name in a new session's history.
    let restored: Vec<String> = fresh.history().iter().map(ChatEntry::text).collect();
    assert_eq!(restored, vec!["first instruction", "second instruction"]);
    let fresh_ids: Vec<String> = fresh.history().iter().map(|e| e.id.to_string()).collect();
    for (fresh_id, saved_id) in fresh_ids.iter().zip(&saved_ids) {
        assert_ne!(fresh_id, saved_id, "a saved entry id was replayed");
    }
}

#[rstest::rstest]
#[test]
fn restoring_pins_keeps_each_saved_pin_position() {
    // Given a saved entry whose pins sit at different positions.
    let session = composed_attendant();
    let entry = saved_entry::entry_for_session("nightly".to_owned(), &session);

    // When restoring them.
    let parent = ChatSessionState::new();
    let mut fresh = ChatSessionState::new_attendant(&parent, true);
    saved_entry::restore_pins(&mut fresh, &entry.pins);

    // Then each entry is pinned where it was saved.
    let positions: Vec<Option<PinPosition>> = fresh
        .history()
        .iter()
        .map(ChatEntry::pin_position)
        .collect();
    assert_eq!(
        positions,
        vec![Some(PinPosition::Top), Some(PinPosition::Bottom)]
    );
}
