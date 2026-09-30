//! Tests for the saved-attendant conversions: what a save captures, and
//! what a restore rebuilds.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code"
)]

use std::collections::BTreeSet;

use jinn_attendant_msg::{AttendantBehavior, AttendantTrigger};
use jinn_core_types::{
    ChatEntry, ChatEntryKind, ContextOverride, FilterMode, NameFilter, PinPosition,
    ToolResultStatus,
};
use jinn_preferences_config::schemas::{AttendantPinConfig, AttendantPinRole};
use jinn_session_state::ChatSessionState;

use crate::saved_entry;

/// A titled attendant carrying two pins and one unpinned entry between
/// them, in the configuration a save is expected to capture.
fn composed_attendant() -> ChatSessionState {
    let parent = ChatSessionState::new();
    let mut attendant = ChatSessionState::new_attendant(&parent, true);
    attendant.set_title("nightly".to_owned());
    attendant.set_attendant_behavior(AttendantBehavior::Reset);
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

    // Then behavior, trigger, and seed template come across as saved, so
    // a created attendant is live from its first run.
    assert_eq!(entry.behavior, AttendantBehavior::Reset);
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

    // Then both instructions are stored in order and the chatter is not.
    let texts: Vec<&str> = entry.pins.iter().map(|pin| pin.text.as_str()).collect();
    assert_eq!(texts, vec!["first instruction", "second instruction"]);
}

#[rstest::rstest]
#[test]
fn an_entry_does_not_store_where_each_pin_sat() {
    // Given an attendant whose pins sit at different positions.
    let session = composed_attendant();

    // When building its entry.
    let entry = saved_entry::entry_for_session("nightly".to_owned(), &session);

    // Then no instruction carries a position at all — a restored attendant
    // is a new session, and where an instruction sat in the old one says
    // nothing about where it should sit here. Restore attaches `Relative`
    // and the session store owns it from then on.
    //
    // The serialized form is asserted at the schema level; here the claim is
    // simply that the captured pins are two bare instructions.
    let pins: Vec<AttendantPinConfig> = entry.pins.clone();
    assert_eq!(
        pins,
        vec![
            AttendantPinConfig {
                role: AttendantPinRole::User,
                text: "first instruction".to_owned(),
            },
            AttendantPinConfig {
                role: AttendantPinRole::User,
                text: "second instruction".to_owned(),
            },
        ]
    );
}

#[rstest::rstest]
#[test]
fn an_entry_records_the_session_tool_filter() {
    // Given an attendant carrying a tool filter.
    let mut session = composed_attendant();
    session.profile_mut().tool_filter = NameFilter::deny(["write".to_owned(), "bash".to_owned()]);

    // When building its entry.
    let entry = saved_entry::entry_for_session("nightly".to_owned(), &session);

    // Then it is stored sorted, so the file does not churn between saves of
    // an unchanged attendant.
    let filter = entry.tool_filter.as_ref().expect("filter recorded");
    assert_eq!(
        filter.names.iter().map(String::as_str).collect::<Vec<_>>(),
        vec!["bash", "write"]
    );
}

#[rstest::rstest]
#[test]
fn an_entry_records_an_allow_mode_filter_with_its_mode() {
    // Given an attendant restricted to two tools by an allow filter.
    let mut session = composed_attendant();
    session.profile_mut().tool_filter = NameFilter {
        mode: FilterMode::Allow,
        names: BTreeSet::from(["read".to_owned(), "mcp__github__*".to_owned()]),
    };

    // When building its entry.
    let entry = saved_entry::entry_for_session("narrow".to_owned(), &session);

    // Then the mode survives into the entry — this is what a plain name list
    // could not carry, and the reason the field is a filter.
    let filter = entry.tool_filter.as_ref().expect("filter recorded");
    assert_eq!(filter.mode, FilterMode::Allow);
    assert!(filter.permits("mcp__github__create_pr"));
    assert!(!filter.permits("bash"));
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
    // the default, and both filters absent — every one of which create
    // inherits back from the parent session.
    assert!(entry.configured_model().is_none());
    assert!(entry.persona_name.is_none());
    assert!(entry.tool_filter.is_none());
    assert!(entry.skill_filter.is_none());
}

#[rstest::rstest]
#[test]
fn restoring_pins_gives_every_instruction_its_own_id() {
    // Given a saved entry's pins, restored twice into two fresh attendants.
    let session = composed_attendant();
    let entry = saved_entry::entry_for_session("nightly".to_owned(), &session);
    let parent = ChatSessionState::new();
    let mut first = ChatSessionState::new_attendant(&parent, true);
    let mut second = ChatSessionState::new_attendant(&parent, true);
    saved_entry::restore_pins(&mut first, &entry.pins);
    saved_entry::restore_pins(&mut second, &entry.pins);

    // Then no two entries share an id — a shared id would make every
    // id-keyed view state (the selected entry, the streaming target) resolve
    // to whichever entry happened to be looked up first.
    let ids: Vec<String> = first
        .history()
        .iter()
        .chain(second.history())
        .map(|e| e.id.to_string())
        .collect();
    let unique: std::collections::HashSet<&String> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len(), "an entry id was replayed");
}

#[rstest::rstest]
#[test]
fn restoring_pins_pins_every_instruction_relative() {
    // Given a saved entry whose pins sat at different positions.
    let session = composed_attendant();
    let entry = saved_entry::entry_for_session("nightly".to_owned(), &session);

    // When restoring them.
    let parent = ChatSessionState::new();
    let mut fresh = ChatSessionState::new_attendant(&parent, true);
    saved_entry::restore_pins(&mut fresh, &entry.pins);

    // Then each is pinned, and pinned relative. A pin is what makes an
    // instruction survive a run with a reset behavior, which force-excludes
    // everything that is not pinned.
    let positions: Vec<Option<PinPosition>> = fresh
        .history()
        .iter()
        .map(ChatEntry::pin_position)
        .collect();
    assert_eq!(
        positions,
        vec![Some(PinPosition::Relative), Some(PinPosition::Relative)]
    );
}

#[rstest::rstest]
#[test]
fn restoring_pins_keeps_each_instruction_in_saved_order() {
    // Given a saved entry whose pins were in a known order.
    let session = composed_attendant();
    let entry = saved_entry::entry_for_session("nightly".to_owned(), &session);

    // When restoring them.
    let parent = ChatSessionState::new();
    let mut fresh = ChatSessionState::new_attendant(&parent, true);
    saved_entry::restore_pins(&mut fresh, &entry.pins);

    // Then they arrive in that order, because a saved attendant's context
    // is a sequence.
    let restored: Vec<String> = fresh.history().iter().map(ChatEntry::text).collect();
    assert_eq!(restored, vec!["first instruction", "second instruction"]);
}

#[rstest::rstest]
#[case(AttendantPinRole::User)]
#[case(AttendantPinRole::Assistant)]
fn a_saved_instruction_restores_as_the_role_it_was_saved_as(#[case] role: AttendantPinRole) {
    // Given a saved pin of this role.
    let pin = AttendantPinConfig {
        role,
        text: "same words either way".to_owned(),
    };

    // When restoring it into a fresh attendant.
    let parent = ChatSessionState::new();
    let mut fresh = ChatSessionState::new_attendant(&parent, true);
    saved_entry::restore_pins(&mut fresh, std::slice::from_ref(&pin));

    // Then the entry comes back as that role. Identical text in the other
    // role would be a different message to the model: an agent result read
    // as an instruction the user gave.
    let kind = &fresh.history()[0].kind;
    let restored_role = match kind {
        ChatEntryKind::User { .. } => AttendantPinRole::User,
        ChatEntryKind::Assistant(_) => AttendantPinRole::Assistant,
        other => panic!("restored as an unexpected kind: {other:?}"),
    };
    assert_eq!(restored_role, role);
}

#[rstest::rstest]
#[test]
fn a_session_with_pinned_non_message_entries_saves_only_its_instructions() {
    // Given an attendant with a pinned tool result and a pinned system line
    // alongside its instructions.
    let session = {
        let parent = ChatSessionState::new();
        let mut attendant = ChatSessionState::new_attendant(&parent, true);
        attendant.push_entry(ChatEntry {
            pin_position: Some(PinPosition::Top),
            ..ChatEntry::user("an instruction")
        });
        attendant.push_entry(ChatEntry {
            pin_position: Some(PinPosition::Relative),
            ..ChatEntry::tool_result(
                "call-1",
                "read_file",
                "contents".to_owned(),
                ToolResultStatus::Success,
            )
        });
        attendant.push_entry(ChatEntry {
            pin_position: Some(PinPosition::Relative),
            ..ChatEntry::system("a note")
        });
        attendant
    };

    // When building its entry.
    let entry = saved_entry::entry_for_session("mixed".to_owned(), &session);

    // Then only the instruction is stored. A tool result is a snapshot of a
    // file as it was; re-injected into a newly spawned attendant it asserts
    // stale contents as current fact. System lines never reach the model.
    let texts: Vec<&str> = entry.pins.iter().map(|pin| pin.text.as_str()).collect();
    assert_eq!(texts, vec!["an instruction"]);
}

#[rstest::rstest]
#[test]
fn an_entry_with_no_pins_restores_an_attendant_with_no_pins() {
    // Given a saved entry carrying no pins.
    let entry = saved_entry::entry_for_session("bare".to_owned(), &ChatSessionState::new());

    // When restoring them.
    let parent = ChatSessionState::new();
    let mut fresh = ChatSessionState::new_attendant(&parent, true);
    saved_entry::restore_pins(&mut fresh, &entry.pins);

    // Then the attendant has an empty history.
    assert!(fresh.history().is_empty());
}

#[rstest::rstest]
#[test]
fn a_restored_instruction_survives_a_reset_run() {
    // Given an attendant restored from a saved entry.
    let session = composed_attendant();
    let entry = saved_entry::entry_for_session("nightly".to_owned(), &session);
    let parent = ChatSessionState::new();
    let mut fresh = ChatSessionState::new_attendant(&parent, true);
    saved_entry::restore_pins(&mut fresh, &entry.pins);
    fresh.push_entry(ChatEntry::user("chatter after the restore"));

    // When the context is reset, which is what a reset-behavior run does.
    let excluded = crate::activation::reset_context(&mut fresh);

    // Then the instructions are still in context while the chatter is not —
    // reset force-excludes everything unpinned, so a restored instruction
    // only survives because restore pinned it.
    let in_context: Vec<String> = fresh
        .history()
        .iter()
        .filter(|entry| entry.context_override != ContextOverride::ForcedExclude)
        .map(ChatEntry::text)
        .collect();
    assert_eq!(in_context, vec!["first instruction", "second instruction"]);
    // And the only entry the reset touched is the chatter.
    let chatter = &fresh.history()[2];
    assert_eq!(excluded, vec![chatter.id.clone()]);
}
