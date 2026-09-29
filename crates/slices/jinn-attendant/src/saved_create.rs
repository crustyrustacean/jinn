//! Creating an attendant from a saved entry.
//!
//! The created attendant takes its *configured* fields from the entry and
//! inherits the rest of its environment from the session it is created
//! under — the same environment `N` hands a fresh attendant, and the same
//! persistence the fresh path performs. Nothing here reaches into the
//! sidebar or the session store: the commands it emits are the ordinary
//! ones an attendant creation publishes.

use jinn_core_types::{ChatEntry, ModelSelection};
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
/// The activation and trigger come across unchanged, so a saved live
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
        if !entry.disabled_tools.is_empty() {
            profile.disabled_tools = entry.disabled_tools.iter().cloned().collect();
        }
        if !entry.disabled_skills.is_empty() {
            profile.disabled_skills = entry.disabled_skills.iter().cloned().collect();
        }
        if let Some(effort) = entry.reasoning_effort {
            profile.reasoning_effort = Some(effort);
        }
    }
    // An endpoint pin is model-specific, so it is only kept when the model
    // it was pinned for is the one the attendant will run. An alloy rotates
    // across models and has no single endpoint; `set_model` drops the pin
    // for exactly that case, and the same rule is applied here rather than
    // attaching a stale endpoint to a rotating set.
    if let Some(endpoint) = &entry.endpoint
        && !matches!(attendant.profile().model, ModelSelection::Alloy { .. })
    {
        attendant.profile_mut().endpoint = Some(endpoint.clone());
    }
    attendant.set_attendant_activation(entry.activation);
    attendant.set_attendant_trigger(entry.trigger);
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
