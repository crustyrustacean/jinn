//! Attendant key actions — `N` (new attendant) and `R` (re-run) in the
//! sessions section.
//!
//! `R` calls into the attendant slice's rerun, which owns the shared
//! fire sequence; the sidebar only resolves the highlighted session and
//! publishes what the rerun produces. `N` creates an attendant of the
//! highlighted session and makes it active, landing in seed mode so the
//! user composes its instructions before anything can fire.

use jinn_kernel::common::app_state::AppState;
use jinn_kernel::protocol::{ChatEntry, IntentResult};
use jinn_session_lifecycle_msg::event::SessionCreated;
use jinn_session_store_msg::PersistSession;
use jinn_slices::ConfigLayer;

use super::close::validate_session_close;

/// The idle session under the sessions-section cursor, if any.
///
/// Same guard the other session actions use: the action only means
/// something with a selected, loaded, idle session row.
fn selected_idle_session(state: &AppState) -> Option<jinn_core_types::SessionId> {
    validate_session_close(state).ok()?;
    state
        .frontend
        .with_sections(|sections| sections.sessions.selected_id.clone(), || None)
}

/// `N` — creates an attendant of the highlighted session and activates it.
///
/// The attendant starts in
/// [`jinn_attendant_msg::AttendantBehavior::Reset`]: submissions pin into
/// its context without dispatching, and its trigger is inert until the user
/// flips it out of seed. Nothing is sent to any provider from this path.
///
/// The attendant inherits the parent's environment (profile, cwd, project,
/// home, enabled MCP servers) via `new_attendant`, plus the config-seeded
/// tool/skill disablements every other creation path applies.
pub fn handle_new_attendant(state: &mut AppState, config: &ConfigLayer) -> IntentResult {
    use jinn_session_msg::SessionSeed;

    let Some(parent_id) = selected_idle_session(state) else {
        return IntentResult::empty();
    };

    // Seed per-session defaults from jinn.toml (disablement sets +
    // auto-enabled MCP servers), matching every other session-creation path.
    let seed = SessionSeed::from_config(config);

    let attendant = {
        // `new_attendant` takes the parent by reference and copies its
        // environment; clone the parent out of the map so the borrow ends
        // before the insert below.
        let Some(parent) = state.session.get(&parent_id).cloned() else {
            return IntentResult::empty();
        };
        // An attendant is an ordinary session: it is saved on creation and
        // again once a turn lands, exactly like a subagent's child.
        let mut attendant = jinn_session_state::ChatSessionState::new_attendant(&parent, true);
        {
            let p = attendant.profile_mut();
            p.tool_filter.clone_from(&seed.tool_filter);
            p.skill_filter.clone_from(&seed.skill_filter);
        }
        attendant
    };

    let attendant_id = attendant.session_id().clone();
    let attendant_cwd = attendant.cwd().to_path_buf();
    state.session.insert(attendant);
    state.session.set_active(attendant_id.clone());
    state.frontend.scope_clear_overlays();
    state.frontend.scope_push(jinn_slices::FocusScope::Input);

    // The attendant is meaningfully created even though no keystroke has
    // landed in it, so it survives a save that runs before its first turn
    // (the `task` tool's `build_child` marks its child for the same reason).
    if let Some(a) = state.session.get_mut(&attendant_id) {
        a.mark_interacted();
    }

    // The parent earns the same mark. A brand new session is not written
    // when it is created — it stays in memory until something makes it
    // worth keeping — and the store discards any snapshot for a session it
    // does not consider persistable, so asking to save an unwritten parent
    // achieves nothing on its own. Attaching an attendant is that
    // something: the child names the parent in its own row, so a parent
    // that is never saved is an attendant pointing at a session the store
    // has never heard of.
    if let Some(p) = state.session.get_mut(&parent_id) {
        p.mark_interacted();
    }

    let mut result = IntentResult::empty()
        // The parent first. A brand new session is not written when it is
        // created, so it exists only in memory until something forces it
        // out. The attendant's own row names the parent, so saving the
        // child while the parent is unwritten is an attendant pointing at
        // a session the store has never heard of — a tree with a hole
        // where the trunk should be. Saving the parent is idempotent, and
        // a no-op for the ordinary case where it was already written.
        .with_message(PersistSession {
            session_id: parent_id.clone(),
        })
        .with_message(PersistSession {
            session_id: attendant_id.clone(),
        })
        .with_message(SessionCreated {
            session_id: attendant_id.clone(),
            cwd: attendant_cwd,
        })
        .with_message(jinn_session_history_msg::PushChatEntry {
            session_id: attendant_id.clone(),
            entry: ChatEntry::system(
                "🛰️ Attendant created in prep mode for providing instructions. When you are done, select the attendant in the sidebar and press `P` to turn prep mode off. The attendant will not run until you do."
            ),
            pin: None,
        });

    if seed.has_auto_enabled_mcp() {
        result = result.with_message(jinn_mcp_msg::McpEnablementChanged {
            session_id: attendant_id,
            enabled: seed.enabled_mcp,
        });
    }
    result
}

/// `R` — re-runs the highlighted attendant.
///
/// Delegates to the attendant slice's rerun (the same sequence a trigger
/// fire uses), then publishes the cancel and dispatch it produces. A
/// no-op on a non-attendant row, or on an attendant still in seed mode;
/// a blocked rerun surfaces its reason as a transient system line.
pub fn handle_rerun_attendant(state: &mut AppState) -> IntentResult {
    let Some(attendant_id) = selected_idle_session(state) else {
        return IntentResult::empty();
    };
    let Some((cancel, dispatch, reset)) =
        jinn_attendant::rerun::rerun_in_state(state, &attendant_id)
    else {
        // Surface the block reason when it is one the user can act on.
        if let Some(reason) = jinn_attendant::rerun::rerun_blocked_reason_in(state, &attendant_id) {
            let session_id = state.session.active_session_id().clone();
            return IntentResult::empty().with_message(jinn_session_history_msg::PushChatEntry {
                session_id,
                entry: ChatEntry::system(format!("⚠️ Cannot re-run: {reason}")),
                pin: None,
            });
        }
        return IntentResult::empty();
    };
    let mut result = IntentResult::empty();
    if let Some(cancel) = cancel {
        result = result.with_message(cancel);
    }
    // The confirmed-cancel cascade: every attendant or subagent beneath this
    // one stops with it, recursively. A trigger deliberately does not do
    // this — it cannot know which descendant the user would want stopped —
    // but `R` is the user saying "start over", and the descendants only exist
    // to answer the question this attendant is re-asking. Forks are
    // boundaries; the walk stops there.
    let mut visited = std::collections::HashSet::new();
    visited.insert(attendant_id.clone());
    result = result.merge(jinn_kernel::feat::intent::cancel::cascade_descendants(
        state,
        &attendant_id,
        &mut visited,
    ));
    if let Some(dispatch) = dispatch {
        result = result.with_message(dispatch);
    }
    // A `Reset` that excluded something changed what the model will see, and
    // that only survives a restart if it is written now — an attendant
    // reloaded from disk would otherwise come back with its full history.
    if !reset.is_empty() {
        result = result.with_message(PersistSession {
            session_id: attendant_id,
        });
    }
    result
}
