//! Bot key actions — `b` (new bot of a session), `B` (standalone bot),
//! and `R` (re-run) in the sessions section.
//!
//! `R` calls into the bot slice's rerun, which owns the shared
//! fire sequence; the sidebar only resolves the highlighted session and
//! publishes what the rerun produces. `b` creates a bot of the
//! highlighted session and makes it active, landing in seed mode so the
//! user composes its instructions before anything can fire. `B` creates
//! a standalone bot — no parent — from the highlighted session's
//! environment, landing in the same seed mode.

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

/// `b` — creates a bot of the highlighted session and activates it.
///
/// The bot starts in
/// [`jinn_bot_msg::BotBehavior::Reset`]: submissions pin into
/// its context without dispatching, and its trigger is inert until the user
/// flips it out of seed. Nothing is sent to any provider from this path.
///
/// The bot inherits the parent's environment (profile, cwd, project,
/// home, enabled MCP servers) via `new_bot`, plus the config-seeded
/// tool/skill disablements every other creation path applies.
pub fn handle_new_bot_of_session(state: &mut AppState, config: &ConfigLayer) -> IntentResult {
    use jinn_session_msg::SessionSeed;

    let Some(parent_id) = selected_idle_session(state) else {
        return IntentResult::empty();
    };

    // Seed per-session defaults from jinn.toml (disablement sets +
    // auto-enabled MCP servers), matching every other session-creation path.
    let seed = SessionSeed::from_config(config);

    let bot = {
        // `new_bot` takes the parent by reference and copies its
        // environment; clone the parent out of the map so the borrow ends
        // before the insert below.
        let Some(parent) = state.session.get(&parent_id).cloned() else {
            return IntentResult::empty();
        };
        // A bot is an ordinary session: it is saved on creation and
        // again once a turn lands, exactly like a subagent's child.
        let mut bot = jinn_session_state::ChatSessionState::new_bot(&parent, true);
        {
            let p = bot.profile_mut();
            p.tool_filter.clone_from(&seed.tool_filter);
            p.skill_filter.clone_from(&seed.skill_filter);
        }
        bot
    };

    let bot_id = bot.session_id().clone();
    let bot_cwd = bot.cwd().clone();
    state.session.insert(bot);
    state.session.set_active(bot_id.clone());
    state.frontend.scope_clear_overlays();
    state.frontend.scope_push(jinn_slices::FocusScope::Input);

    // The bot is meaningfully created even though no keystroke has
    // landed in it, so it survives a save that runs before its first turn
    // (the `spawn_subagent` tool's `build_child` marks its child for the same reason).
    if let Some(a) = state.session.get_mut(&bot_id) {
        a.mark_interacted();
    }

    // The parent earns the same mark. A brand new session is not written
    // when it is created — it stays in memory until something makes it
    // worth keeping — and the store discards any snapshot for a session it
    // does not consider persistable, so asking to save an unwritten parent
    // achieves nothing on its own. Attaching a bot is that
    // something: the child names the parent in its own row, so a parent
    // that is never saved is a bot pointing at a session the store
    // has never heard of.
    if let Some(p) = state.session.get_mut(&parent_id) {
        p.mark_interacted();
    }

    let mut result = IntentResult::empty()
        // The parent first. A brand new session is not written when it is
        // created, so it exists only in memory until something forces it
        // out. The bot's own row names the parent, so saving the
        // child while the parent is unwritten is a bot pointing at
        // a session the store has never heard of — a tree with a hole
        // where the trunk should be. Saving the parent is idempotent, and
        // a no-op for the ordinary case where it was already written.
        .with_message(PersistSession {
            session_id: parent_id.clone(),
        })
        .with_message(PersistSession {
            session_id: bot_id.clone(),
        })
        .with_message(SessionCreated {
            session_id: bot_id.clone(),
            cwd: bot_cwd,
        })
        .with_message(jinn_session_history_msg::PushChatEntry {
            session_id: bot_id.clone(),
            entry: ChatEntry::system(
                "Bot created in prep mode for providing instructions. When you are done, select the bot in the sidebar and press `P` to turn prep mode off. The bot will not run until you do."
            ),
            pin: None,
        });

    if seed.has_auto_enabled_mcp() {
        result = result.with_message(jinn_mcp_msg::McpEnablementChanged {
            session_id: bot_id,
            enabled: seed.enabled_mcp,
        });
    }
    result
}

/// `B` — creates a standalone bot from the highlighted session's
/// environment and activates it.
///
/// The same sequence [`handle_new_bot_of_session`] runs, minus everything
/// the parent link causes: no parent mark, no parent persist, and a
/// `new_standalone_bot` base whose seed template is empty and whose
/// trigger can never fire. The highlighted session is the environment
/// source only.
pub fn handle_new_standalone_bot(state: &mut AppState, config: &ConfigLayer) -> IntentResult {
    use jinn_session_msg::SessionSeed;

    let Some(source_id) = selected_idle_session(state) else {
        return IntentResult::empty();
    };
    let seed = SessionSeed::from_config(config);

    let bot = {
        let Some(source) = state.session.get(&source_id).cloned() else {
            return IntentResult::empty();
        };
        let mut bot = jinn_session_state::ChatSessionState::new_standalone_bot(&source, true);
        {
            let p = bot.profile_mut();
            p.tool_filter.clone_from(&seed.tool_filter);
            p.skill_filter.clone_from(&seed.skill_filter);
        }
        bot
    };

    let bot_id = bot.session_id().clone();
    let bot_cwd = bot.cwd().clone();
    state.session.insert(bot);
    state.session.set_active(bot_id.clone());
    state.frontend.scope_clear_overlays();
    state.frontend.scope_push(jinn_slices::FocusScope::Input);

    if let Some(a) = state.session.get_mut(&bot_id) {
        a.mark_interacted();
    }

    let mut result = IntentResult::empty()
        .with_message(PersistSession {
            session_id: bot_id.clone(),
        })
        .with_message(SessionCreated {
            session_id: bot_id.clone(),
            cwd: bot_cwd,
        })
        .with_message(jinn_session_history_msg::PushChatEntry {
            session_id: bot_id.clone(),
            entry: ChatEntry::system(
                "Standalone bot created in prep mode. Type its standing instructions, then press `P` to take it out of prep mode. It will not run until you do."
            ),
            pin: None,
        });

    if seed.has_auto_enabled_mcp() {
        result = result.with_message(jinn_mcp_msg::McpEnablementChanged {
            session_id: bot_id,
            enabled: seed.enabled_mcp,
        });
    }
    result
}

/// `R` — re-runs the highlighted bot.
///
/// Delegates to the bot slice's rerun (the same sequence a trigger
/// fire uses), then publishes the cancel and dispatch it produces. A
/// no-op on a non-bot row, or on a bot still in seed mode;
/// a blocked rerun surfaces its reason as a transient system line.
pub fn handle_rerun_bot(state: &mut AppState) -> IntentResult {
    let Some(bot_id) = selected_idle_session(state) else {
        return IntentResult::empty();
    };
    let Some((cancel, dispatch, reset)) = jinn_bot::rerun::rerun_in_state(state, &bot_id) else {
        // Surface the block reason when it is one the user can act on.
        if let Some(reason) = jinn_bot::rerun::rerun_blocked_reason_in(state, &bot_id) {
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
    // The confirmed-cancel cascade: every bot or subagent beneath this
    // one stops with it, recursively. A trigger deliberately does not do
    // this — it cannot know which descendant the user would want stopped —
    // but `R` is the user saying "start over", and the descendants only exist
    // to answer the question this bot is re-asking. Forks are
    // boundaries; the walk stops there.
    let mut visited = std::collections::HashSet::new();
    visited.insert(bot_id.clone());
    result = result.merge(jinn_kernel::feat::intent::cancel::cascade_descendants(
        state,
        &bot_id,
        &mut visited,
    ));
    if let Some(dispatch) = dispatch {
        result = result.with_message(dispatch);
    }
    // A `Reset` that excluded something changed what the model will see, and
    // that only survives a restart if it is written now — a bot
    // reloaded from disk would otherwise come back with its full history.
    if !reset.is_empty() {
        result = result.with_message(PersistSession { session_id: bot_id });
    }
    result
}
