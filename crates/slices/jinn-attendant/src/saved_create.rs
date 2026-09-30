//! Creating an attendant from a saved entry.
//!
//! The created attendant takes its *configured* fields from the entry and
//! inherits the rest of its environment from the session it is created
//! under — the same environment `N` hands a fresh attendant, and the same
//! persistence the fresh path performs. Nothing here reaches into the
//! sidebar or the session store: the commands it emits are the ordinary
//! ones an attendant creation publishes.

use jinn_core_types::ChatEntry;
use jinn_preferences_config::schemas::AttendantEntryConfig;
use jinn_session_lifecycle_msg::event::SessionCreated;
use jinn_session_state::ChatSessionState;
use jinn_session_store_msg::PersistSession;
use jinn_slices::RouteResult as IntentResult;

use crate::saved_entry;

/// The line the created attendant opens with, naming what it is.
const CREATED_NOTICE: &str =
    "🛰️ Attendant restored from a saved entry — its saved pins and configuration are in place.";

/// Builds the attendant `entry` describes, as a child of `parent`.
///
/// Every field the entry carries is applied verbatim; every field it does
/// not carry is whatever `new_attendant` inherited from the parent, which
/// is the whole point of "config first, environment inherited": an entry
/// saved on one machine must still run where the user's cwd and project
/// are.
///
/// The behavior, trigger, and prep mode come across unchanged, so a live
/// attendant starts live rather than dropping back into seed mode.
#[must_use]
pub fn build(entry: &AttendantEntryConfig, parent: &ChatSessionState) -> ChatSessionState {
    let mut attendant = ChatSessionState::new_attendant(parent, true);
    {
        let profile = attendant.profile_mut();
        if let Some(model) = entry.configured_model() {
            profile.model = model.clone();
        }
        if let Some(persona) = &entry.persona_name {
            profile.persona_name = persona.clone();
        }
        // An absent filter leaves the creating session's in place, so an
        // attendant saved with no filter configured still gets whatever its
        // parent had. A present filter replaces it wholesale — including an
        // allow-mode one, which is the whole point of being able to save
        // one, and including one naming nothing, which is an attendant with
        // no tools at all.
        if let Some(filter) = &entry.tool_filter {
            profile.tool_filter = Some(filter.clone());
        }
        if let Some(filter) = &entry.skill_filter {
            profile.skill_filter = Some(filter.clone());
        }
        if let Some(effort) = entry.reasoning_effort {
            profile.reasoning_effort = Some(effort);
        }
    }
    attendant.set_attendant_behavior(entry.behavior);
    attendant.set_attendant_trigger(entry.trigger);
    attendant.set_attendant_is_prepping(entry.prep_mode);
    attendant.set_seed_template(entry.seed_template.clone());
    // The entry's name is the attendant's identity, both in the picker and
    // in the sessions list.
    attendant.set_title(entry.name.clone());
    saved_entry::restore_pins(&mut attendant, &entry.pins);
    attendant
}

/// The messages a creation publishes, in the order they must arrive.
///
/// The parent is persisted first: an attendant's own row names the parent,
/// and a parent that has never been written is an attendant pointing at a
/// session the store has never heard of.
#[must_use]
pub fn creation_messages(
    parent_id: &jinn_core_types::SessionId,
    attendant: &ChatSessionState,
) -> IntentResult {
    let attendant_id = attendant.session_id().clone();
    let cwd = attendant.cwd().to_path_buf();
    IntentResult::empty()
        .with_message(PersistSession {
            session_id: parent_id.clone(),
        })
        .with_message(PersistSession {
            session_id: attendant_id.clone(),
        })
        .with_message(SessionCreated {
            session_id: attendant_id.clone(),
            cwd,
        })
        .with_message(jinn_session_history_msg::PushChatEntry {
            session_id: attendant_id,
            entry: ChatEntry::system(CREATED_NOTICE),
            pin: None,
        })
}

/// Creates `entry` on `state` under the active session, publishing the
/// creation's messages and making the new attendant active.
///
/// Returns `None` when the active session is gone or is itself an
/// attendant: an attendant of an attendant is not a thing this feature
/// creates, and silently doing it would produce a lineage the trigger
/// actor has no rules for.
pub fn create_in_state(
    state: &mut jinn_app_state::AppState,
    entry: &AttendantEntryConfig,
) -> Option<IntentResult> {
    let parent_id = state.session.active_session_id().clone();
    let parent = state.session.get(&parent_id)?.clone();
    if parent.is_attendant() {
        return None;
    }

    let attendant = build(entry, &parent);
    let attendant_id = attendant.session_id().clone();
    state.session.insert(attendant);
    state.session.set_active(attendant_id.clone());
    // The new attendant is created, not typed into: it is worth keeping
    // before it has said anything, exactly as a fresh `N` attendant is.
    if let Some(a) = state.session.get_mut(&attendant_id) {
        a.mark_interacted();
    }
    if let Some(p) = state.session.get_mut(&parent_id) {
        p.mark_interacted();
    }
    state.frontend.scope_clear_overlays();
    state.frontend.scope_push(jinn_slices::FocusScope::Input);
    Some(creation_messages(
        &parent_id,
        state.session.get(&attendant_id)?,
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test code")]

    use jinn_core_types::{FilterMode, ModelSelection, NameFilter};
    use jinn_preferences_config::schemas::AttendantEntryConfig;
    use jinn_session_state::ChatSessionState;

    use super::build;

    /// The mode of the attendant's tool filter, which must be present for
    /// every test here that asserts on it.
    fn filter_mode(attendant: &ChatSessionState) -> FilterMode {
        attendant
            .tool_filter()
            .expect("attendant carries a tool filter")
            .mode
    }

    /// An entry carrying the given filters and nothing else.
    fn entry_with(
        tool_filter: Option<NameFilter>,
        skill_filter: Option<NameFilter>,
    ) -> AttendantEntryConfig {
        AttendantEntryConfig {
            name: "nightly".to_owned(),
            tool_filter,
            skill_filter,
            ..AttendantEntryConfig::default()
        }
    }

    #[rstest::rstest]
    fn a_created_attendant_carries_the_saved_tool_filter() {
        // Given a parent withholding two tools, and an entry that saved so.
        let mut parent = ChatSessionState::new();
        parent.set_tool_filter(Some(NameFilter::deny([
            "bash".to_owned(),
            "write".to_owned(),
        ])));

        // When an attendant is created from it.
        let attendant = build(
            &entry_with(
                Some(NameFilter::deny(["bash".to_owned(), "write".to_owned()])),
                None,
            ),
            &parent,
        );

        // Then the created attendant withholds the same tools, so a save and
        // restore is a no-op on its access rather than a widening of it.
        assert!(!attendant.is_tool_enabled("bash"));
        assert!(!attendant.is_tool_enabled("write"));
        assert!(attendant.is_tool_enabled("read"));
    }

    #[rstest::rstest]
    fn a_created_attendant_carries_a_saved_allow_filter_intact() {
        // Given an entry restricted to two tools by an allow filter.
        let parent = ChatSessionState::new();
        let filter = NameFilter {
            mode: FilterMode::Allow,
            names: ["read".to_owned(), "mcp__github__*".to_owned()]
                .into_iter()
                .collect(),
        };

        // When an attendant is created from it.
        let attendant = build(&entry_with(Some(filter.clone()), None), &parent);

        // Then the mode survives. Restoring it as a deny filter would invert
        // the attendant's tool access — it would be left able to run
        // everything except the two tools it was meant to be limited to.
        assert_eq!(filter_mode(&attendant), FilterMode::Allow);
        assert!(attendant.is_tool_enabled("mcp__github__create_pr"));
        assert!(!attendant.is_tool_enabled("bash"));
    }

    #[rstest::rstest]
    fn a_created_attendant_inherits_the_parents_filter_when_none_was_saved() {
        // Given a parent withholding a tool and an entry that saved no filter.
        let mut parent = ChatSessionState::new();
        parent.set_tool_filter(Some(NameFilter::deny(["bash".to_owned()])));

        // When an attendant is created from it.
        let attendant = build(&entry_with(None, None), &parent);

        // Then it inherits. An absent filter means "as configured here",
        // which is what an attendant saved with nothing overrides did before
        // filters existed.
        assert!(!attendant.is_tool_enabled("bash"));
    }

    #[rstest::rstest]
    fn a_created_attendant_inherits_the_parents_allow_filter_when_none_was_saved() {
        // Given a parent restricted by an allow filter, and an entry with none.
        let mut parent = ChatSessionState::new();
        parent.set_tool_filter(Some(NameFilter {
            mode: FilterMode::Allow,
            names: ["read".to_owned()].into_iter().collect(),
        }));

        // When an attendant is created from it.
        let attendant = build(&entry_with(None, None), &parent);

        // Then the restriction is inherited whole, mode included — inheriting
        // only the withheld names would leave the child permitted everything.
        assert_eq!(filter_mode(&attendant), FilterMode::Allow);
        assert!(!attendant.is_tool_enabled("bash"));
    }

    #[rstest::rstest]
    fn a_created_attendant_carries_the_saved_skill_filter() {
        // Given a parent withholding a skill, and an entry that saved so.
        let mut parent = ChatSessionState::new();
        parent.set_skill_filter(Some(NameFilter::deny(["scream".to_owned()])));

        // When an attendant is created from it.
        let attendant = build(
            &entry_with(None, Some(NameFilter::deny(["dataviz".to_owned()]))),
            &parent,
        );

        // Then the saved skill filter replaces the parent's rather than
        // composing with it.
        assert!(!attendant.is_skill_enabled("dataviz"));
        assert!(
            attendant.is_skill_enabled("scream"),
            "the saved filter replaces the parent's, it does not merge with it"
        );
    }

    #[rstest::rstest]
    fn a_created_attendant_inherits_the_parents_untouched_fields() {
        // Given a parent on a specific model.
        let mut parent = ChatSessionState::new();
        parent.set_model(ModelSelection::Single("zai/glm-4.7".to_owned()));

        // When an attendant is created from an entry that configures no model.
        let attendant = build(&entry_with(None, None), &parent);

        // Then it runs on the parent's model, as an entry without one always
        // did — the filter work must not have changed the inheritance path.
        assert_eq!(
            attendant.model_selection(),
            &ModelSelection::Single("zai/glm-4.7".to_owned())
        );
    }
}
