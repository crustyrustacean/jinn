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
use super::state::sorted_open_sessions;

/// The idle session under the sessions-section cursor, if any.
///
/// Same guard the other session actions use: the action only means
/// something with a selected, loaded, idle session row.
fn selected_idle_session(state: &AppState) -> Option<jinn_core_types::SessionId> {
    validate_session_close(state).ok()?;
    let index = state
        .frontend
        .with_sections(|sections| sections.sessions.selected_index, || None)?;
    sorted_open_sessions(state)
        .get(index)
        .map(|entry| entry.id.clone())
}

/// `N` — creates an attendant of the highlighted session and activates it.
///
/// The attendant starts in
/// [`jinn_attendant_msg::AttendantActivation::Seed`]: submissions pin into
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
            p.disabled_tools.clone_from(&seed.disabled_tools);
            p.disabled_skills.clone_from(&seed.disabled_skills);
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

    let mut result = IntentResult::empty()
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
                "🛰️ Attendant created in seed mode — compose its instructions, \
                 then flip activation to fire.",
            ),
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
    let Some((cancel, dispatch)) = jinn_attendant::rerun::rerun_in_state(state, &attendant_id)
    else {
        // Surface the block reason when it is one the user can act on.
        if let Some(reason) = jinn_attendant::rerun::rerun_blocked_reason_in(state, &attendant_id) {
            let session_id = state.session.active_session_id().clone();
            return IntentResult::empty().with_message(jinn_session_history_msg::PushChatEntry {
                session_id,
                entry: ChatEntry::system(format!("⚠️ Cannot re-run: {reason}")),
            });
        }
        return IntentResult::empty();
    };
    let mut result = IntentResult::empty();
    if let Some(cancel) = cancel {
        result = result.with_message(cancel);
    }
    if let Some(dispatch) = dispatch {
        result = result.with_message(dispatch);
    }
    result
}
