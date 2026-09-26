//! Session lifecycle setup and teardown intent handlers.
//!
//! These handlers bridge the Intent-driven architecture with the session lifecycle
//! system. The IntentHandler calls these functions directly; they mutate `AppState`
//! and return `IntentResult` with commands for the actor system.

use crate::common::app_state::AppState;
use crate::protocol::IntentResult;
use jinn_config::ConfigLayer;
use jinn_core_types::{DEFAULT_PERSONA_NAME, SessionId, SessionProfile};
use jinn_preferences_config::schemas::SessionLifecycle;
use jinn_session_history_msg::PushChatEntry;
use jinn_session_lifecycle_msg::command::{RunSessionSetup, RunSessionTeardown};
use jinn_session_lifecycle_msg::event::SessionCreated;
use jinn_session_lifecycle_msg::{CommandTemplate, setup_running_msg};
use jinn_session_msg::SessionSeed;
use jinn_session_state::ChatSessionState;
use jinn_session_store_msg::PersistSession;

/// Handle `Intent::SessionLifecycleSetup`.
///
/// Creates a new session from the named lifecycle. If the lifecycle has a
/// `setup_command`, emits `Command::RunSessionSetup` for async execution.
/// If no setup command (blank or blank-like lifecycle), creates the session
/// with the default CWD immediately.
pub fn handle_session_lifecycle_setup(
    state: &mut AppState,
    lifecycle_name: &str,
    args: &[String],
    cwd: Option<&std::path::Path>,
    config: &jinn_config::ConfigLayer,
) -> IntentResult {
    // Extract setup command before mutating state (borrow checker).
    let setup_command = find_lifecycle(config, lifecycle_name).and_then(|l| l.setup.clone());

    let model = state
        .frontend
        .app_state
        .last_model
        .clone()
        .unwrap_or_default();

    let persona_name = state
        .persona_selection()
        .and_then(|cell| cell.read().active.clone())
        .unwrap_or_else(|| DEFAULT_PERSONA_NAME.to_owned());

    let reasoning_effort = state.frontend.app_state.reasoning_effort;

    // Seed per-session defaults from jinn.toml (disablement sets +
    // auto-enabled MCP servers), matching every other session-creation path.
    let seed = SessionSeed::from_config(config);

    let mut new_session = ChatSessionState::new_with_profile(SessionProfile::new(
        model,
        persona_name,
        seed.disabled_tools.clone(),
        seed.disabled_skills.clone(),
        reasoning_effort,
        None,
    ));
    new_session.set_enabled_mcp_servers(seed.enabled_mcp.clone());
    let new_id = new_session.session_id().clone();

    // Set lifecycle metadata on the session core.
    new_session.set_lifecycle_name(if lifecycle_name.is_empty() {
        None
    } else {
        Some(lifecycle_name.to_owned())
    });
    new_session.set_lifecycle_args(args.to_vec());

    // Resolve the new session's starting CWD and project stamp. The CWD
    // precedence is:
    //   1. explicit `cwd` override (e.g. scripted callers),
    //   2. the stashed creation's starting CWD
    //      (set by the project picker then consumed here),
    //   3. inherit the active session's CWD.
    //
    // The project stamp comes only from the stashed creation - the projects UI
    // is the sole source of a project association. The two are independent on
    // purpose: setup scripts may re-cwd the session later, the project never
    // moves.
    //
    // CWD is resolved BEFORE insert/set_active — once set_active(new_id)
    // runs below, active_session() points at this new session. The stash is
    // always cleared here so it never leaks into the next creation, even when
    // an explicit `cwd` was supplied.
    //
    // A scripted lifecycle's stdout output still wins as the final CWD via the
    // session actor; this only sets the starting value.
    let (stamped_project, starting_cwd) = {
        let pending = state.frontend.pending_creation.take();
        let cwd_override = cwd
            .map(std::path::Path::to_path_buf)
            .or_else(|| pending.as_ref().map(|p| p.starting_cwd.clone()));
        (
            pending.map(|p| p.project_dir),
            cwd_override.unwrap_or_else(|| state.active_session().cwd().to_path_buf()),
        )
    };
    // Defensively clear any residual stash so it can never leak into the next
    // creation (the `.take()` above is skipped only when an explicit cwd
    // overrides, so clear unconditionally here).
    state.frontend.pending_creation = None;
    new_session.set_cwd(starting_cwd.clone());
    new_session.set_project(stamped_project);

    state.session.insert(new_session);
    state.session.set_active(new_id.clone());
    state.frontend.scope_clear_overlays();
    state.frontend.scope_push(jinn_slices::FocusScope::Input);

    // Build the session-created event.
    let created_event = SessionCreated {
        session_id: new_id.clone(),
        cwd: starting_cwd,
    };

    // If the lifecycle has a setup command, emit it for async execution.
    if let Some(ref setup_cmd) = setup_command {
        let rendered = match setup_cmd {
            jinn_preferences_config::schemas::LifecycleCommand::Shell(cmd) => {
                let template = CommandTemplate::parse(cmd);
                if args.is_empty() {
                    cmd.clone()
                } else {
                    template.render(args)
                }
            }
            jinn_preferences_config::schemas::LifecycleCommand::Builtin(id) => id.to_string(),
        };

        let mut result = IntentResult::empty()
            .with_message(PersistSession {
                session_id: new_id.clone(),
            })
            .with_message(PushChatEntry {
                session_id: new_id.clone(),
                entry: setup_running_msg(),
            })
            .with_message(RunSessionSetup {
                session_id: new_id.clone(),
                command: rendered,
                args: args.to_vec(),
                lifecycle_command: Some(setup_cmd.clone()),
            })
            .with_message(created_event);

        // Notify the MCP coordinator of any config-seeded enablement so the
        // new session's servers spawn without a picker visit. When nothing is
        // auto-enabled, the message is skipped (nothing to reconcile).
        if seed.has_auto_enabled_mcp() {
            result = result.with_message(jinn_mcp_msg::McpEnablementChanged {
                session_id: new_id,
                enabled: seed.enabled_mcp,
            });
        }

        return result;
    }

    // No setup command — the starting CWD was already set on the new session
    // before insert (above), so there's nothing more to do here.
    if !seed.has_auto_enabled_mcp() {
        return IntentResult::new_message(created_event);
    }
    IntentResult::new_message(created_event).with_message(jinn_mcp_msg::McpEnablementChanged {
        session_id: new_id,
        enabled: seed.enabled_mcp,
    })
}

/// Handle `Intent::SessionClose`.
///
/// Emits a `CloseSession` command for the active session. The session actor
/// handles teardown, archival, removal, and emits `SessionClosed`.
pub fn handle_session_close(state: &mut AppState) -> IntentResult {
    let closing_id = state.session.active_session_id().clone();
    close_session_and_switch(&closing_id)
}

/// Resolve and render the teardown command for a session by ID.
///
/// Reads the session's `lifecycle_name` + `lifecycle_args`, looks up the named
/// lifecycle in the configuration layer, takes its `teardown` command, and renders
/// it (replaying the stored args). Returns `None` when the session doesn't exist,
/// has no lifecycle name, or the named lifecycle has no teardown command.
///
/// This is the session-ID-keyed counterpart of the sidebar teardown handler —
/// UI-coupled callers resolve the selected session's ID, then delegate here.
pub fn build_run_session_teardown(
    state: &AppState,
    session_id: &SessionId,
    config: &jinn_config::ConfigLayer,
) -> Option<RunSessionTeardown> {
    use jinn_preferences_config::schemas::LifecycleCommand;

    let (teardown_command, lifecycle_args) = {
        let session = state.session.get(session_id)?;
        let lifecycle_name = session.lifecycle_name().map(String::from);
        let args = session.lifecycle_args().to_vec();
        let teardown = lifecycle_name
            .as_deref()
            .and_then(|name| find_lifecycle(config, name))
            .and_then(|lifecycle| lifecycle.teardown);
        (teardown, args)
    };

    let teardown_cmd = teardown_command.as_ref()?;
    let rendered = match teardown_cmd {
        LifecycleCommand::Shell(cmd) => {
            let template = CommandTemplate::parse(cmd);
            if lifecycle_args.is_empty() {
                cmd.clone()
            } else {
                template.render(&lifecycle_args)
            }
        }
        LifecycleCommand::Builtin(id) => id.to_string(),
    };

    Some(RunSessionTeardown {
        session_id: session_id.clone(),
        command: rendered,
        args: lifecycle_args,
    })
}

/// The teardown command of the named lifecycle, read from the
/// `[[session_lifecycle.script]]` section.
#[must_use]
pub fn lifecycle_teardown(
    config: &ConfigLayer,
    name: &str,
) -> Option<jinn_preferences_config::schemas::LifecycleCommand> {
    find_lifecycle(config, name)?.teardown
}

/// Looks up a lifecycle by name in the `[[session_lifecycle.script]]`
/// section.
fn find_lifecycle(config: &ConfigLayer, name: &str) -> Option<SessionLifecycle> {
    config
        .get_list::<SessionLifecycle>()
        .unwrap_or_default()
        .into_iter()
        .find(|lifecycle| lifecycle.name == name)
}

/// Emit a `CloseSession` command to the actor system.
/// The session actor handles actual removal, active session switching, and emits
/// `SessionClosed` for the sidebar actor to clamp the cursor.
fn close_session_and_switch(closing_id: &SessionId) -> IntentResult {
    use jinn_session_lifecycle_msg::CloseSession;
    IntentResult::new_message(CloseSession {
        session_id: closing_id.clone(),
    })
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;
    use crate::common::app_state::AppState;
    use crate::protocol::ChatEntry;

    /// A layer carrying a single lifecycle, as a user's `jinn.toml` holds
    /// one under `[[session_lifecycle.script]]`.
    fn lifecycle_config(name: &str, setup: Option<&str>, teardown: Option<&str>) -> ConfigLayer {
        let setup = setup.map_or_else(String::new, |s| format!("setup_command = \"{s}\"\n"));
        let teardown =
            teardown.map_or_else(String::new, |t| format!("teardown_command = \"{t}\"\n"));
        let document =
            format!("[[session_lifecycle.script]]\nname = \"{name}\"\n{setup}{teardown}");

        jinn_config::testutil::config_layer(&document)
    }

    #[rstest::rstest]
    fn session_lifecycle_setup_with_blank_creates_session() {
        // Given default state (no lifecycles configured).
        let mut state = AppState::default_with_scope_focus();
        // Set the active session's cwd to a distinct value so inheritance is
        // distinguishable from default_cwd() (the app launch dir).
        let inherited_cwd = std::path::PathBuf::from("/tmp/inherited-project");
        state.active_session_mut().set_cwd(inherited_cwd.clone());
        let old_id = state.session.active_session_id().clone();

        // When handling SessionLifecycleSetup with blank lifecycle.
        let result = handle_session_lifecycle_setup(
            &mut state,
            "",
            &[],
            None,
            crate::common::render_ctx::empty_config_layer(),
        );

        // Then a new session is created.
        assert_ne!(*state.session.active_session_id(), old_id);
        // And the old empty session is preserved (no auto-close).
        assert!(state.session.contains(&old_id));
        // And two sessions exist.
        assert_eq!(state.session.session_count(), 2);
        // And one message emitted (SessionCreated).
        assert_eq!(result.message_names.len(), 1);
        assert!(result.message_names[0].contains("SessionCreated"));
        // And the session has no lifecycle name.
        assert!(state.active_session().lifecycle_name().is_none());
        // And the new session inherited the active session's CWD, not the app
        // launch dir.
        assert_eq!(state.active_session().cwd(), inherited_cwd);
    }

    #[rstest::rstest]
    fn explicit_cwd_override_overrides_inherited_cwd() {
        // Given a state whose active session has a distinct CWD and a
        // pending creation stashed on the frontend (as the project picker does).
        let mut state = AppState::default_with_scope_focus();
        state
            .active_session_mut()
            .set_cwd(std::path::PathBuf::from("/tmp/active-project"));
        state.frontend.pending_creation =
            Some(crate::feat::ui::frontend_state::PendingSessionCreation {
                project_dir: std::path::PathBuf::from("/tmp/override-project"),
                starting_cwd: std::path::PathBuf::from("/tmp/override-project"),
            });

        // When handling SessionLifecycleSetup with an explicit cwd override.
        let _result = handle_session_lifecycle_setup(
            &mut state,
            "",
            &[],
            Some(std::path::Path::new("/tmp/explicit-dir")),
            crate::common::render_ctx::empty_config_layer(),
        );

        // Then the new session's CWD is the explicit override, not the active
        // session's CWD and not the stashed starting CWD.
        assert_eq!(
            state.active_session().cwd(),
            std::path::Path::new("/tmp/explicit-dir"),
        );
        // And the stash is cleared (never leaks to the next creation).
        assert!(state.frontend.pending_creation.is_none());
    }

    #[rstest::rstest]
    fn pending_creation_cwd_is_used_when_no_explicit_cwd_given() {
        // Given a state whose active session has a distinct CWD and a
        // pending creation stashed on the frontend.
        let mut state = AppState::default_with_scope_focus();
        state
            .active_session_mut()
            .set_cwd(std::path::PathBuf::from("/tmp/active-project"));
        state.frontend.pending_creation =
            Some(crate::feat::ui::frontend_state::PendingSessionCreation {
                project_dir: std::path::PathBuf::from("/tmp/override-project"),
                starting_cwd: std::path::PathBuf::from("/tmp/override-project"),
            });

        // When handling SessionLifecycleSetup with no explicit cwd.
        let _result = handle_session_lifecycle_setup(
            &mut state,
            "",
            &[],
            None,
            crate::common::render_ctx::empty_config_layer(),
        );

        // Then the new session's CWD is the stashed starting CWD, not the
        // active session's CWD.
        assert_eq!(
            state.active_session().cwd(),
            std::path::Path::new("/tmp/override-project"),
        );
        // And the stash is cleared after consumption.
        assert!(state.frontend.pending_creation.is_none());
    }

    #[rstest::rstest]
    fn setup_stamps_project_from_pending_creation() {
        // Given a state with a pending creation stashed from the projects UI.
        let mut state = AppState::default_with_scope_focus();
        state.frontend.pending_creation =
            Some(crate::feat::ui::frontend_state::PendingSessionCreation {
                project_dir: std::path::PathBuf::from("/home/user/projects/jinn"),
                starting_cwd: std::path::PathBuf::from("/home/user/projects/jinn"),
            });

        // When handling SessionLifecycleSetup.
        let _result = handle_session_lifecycle_setup(
            &mut state,
            "",
            &[],
            None,
            crate::common::render_ctx::empty_config_layer(),
        );

        // Then the new active session is stamped with the stashed project.
        assert_eq!(
            state.active_session().project(),
            Some(std::path::Path::new("/home/user/projects/jinn")),
        );
    }

    #[rstest::rstest]
    fn setup_without_pending_creation_leaves_project_none() {
        // Given a default state with no pending creation stash.
        let mut state = AppState::default_with_scope_focus();

        // When handling SessionLifecycleSetup.
        let _result = handle_session_lifecycle_setup(
            &mut state,
            "",
            &[],
            None,
            crate::common::render_ctx::empty_config_layer(),
        );

        // Then the new active session has no project association.
        assert_eq!(state.active_session().project(), None);
    }

    #[rstest::rstest]
    fn setup_consumes_stash_exactly_once() {
        // Given a state that already consumed a pending creation.
        let mut state = AppState::default_with_scope_focus();
        state.frontend.pending_creation =
            Some(crate::feat::ui::frontend_state::PendingSessionCreation {
                project_dir: std::path::PathBuf::from("/tmp/first-project"),
                starting_cwd: std::path::PathBuf::from("/tmp/first-project"),
            });
        let _result = handle_session_lifecycle_setup(
            &mut state,
            "",
            &[],
            None,
            crate::common::render_ctx::empty_config_layer(),
        );

        // When creating a second session (the stash is now None).
        let _result = handle_session_lifecycle_setup(
            &mut state,
            "",
            &[],
            None,
            crate::common::render_ctx::empty_config_layer(),
        );

        // Then the second session has no project association (no leak from
        // the first creation).
        assert_eq!(state.active_session().project(), None);
        // And the stash is still clear.
        assert!(state.frontend.pending_creation.is_none());
    }

    #[rstest::rstest]
    fn scripted_lifecycle_setup_pre_seeds_inherited_cwd_in_memory() {
        // Given a state whose active session has a distinct CWD, and a
        // lifecycle with a setup_command (so the script path is taken).
        let mut state = AppState::default_with_scope_focus();
        let inherited_cwd = std::path::PathBuf::from("/tmp/inherited-project");
        state.active_session_mut().set_cwd(inherited_cwd.clone());
        let config = lifecycle_config("fossil branch", Some("echo /tmp/workdir"), None);

        // When handling SessionLifecycleSetup with the scripted lifecycle.
        let _result =
            handle_session_lifecycle_setup(&mut state, "fossil branch", &[], None, &config);

        // Then the new session's in-memory CWD is the inherited value
        // (pre-seeded before the actor runs the script). The actor may
        // overwrite it with the script's stdout later, but at creation
        // time the inherited CWD is present.
        assert_eq!(state.active_session().cwd(), inherited_cwd);
    }

    #[rstest::rstest]
    fn session_lifecycle_setup_with_lifecycle_emits_command() {
        // Given a state with a lifecycle that has a setup_command.
        let mut state = AppState::default_with_scope_focus();
        let old_id = state.session.active_session_id().clone();
        let config = lifecycle_config("fossil branch", Some("echo /tmp/workdir"), None);

        // When handling SessionLifecycleSetup.
        let result =
            handle_session_lifecycle_setup(&mut state, "fossil branch", &[], None, &config);

        // Then a new session is created.
        assert_ne!(*state.session.active_session_id(), old_id);
        // And the session has the lifecycle name.
        assert_eq!(
            state.active_session().lifecycle_name(),
            Some("fossil branch")
        );
        // And PersistSession, PushChatEntry, RunSessionSetup, SessionCreated are emitted.
        assert_eq!(result.message_names.len(), 4);
        assert!(result.message_names[0].contains("PersistSession"));
        assert!(result.message_names[1].contains("PushChatEntry"));
        assert!(result.message_names[2].contains("RunSessionSetup"));
        assert!(result.message_names[3].contains("SessionCreated"));
    }

    #[rstest::rstest]
    fn session_lifecycle_setup_with_args_renders_command() {
        // Given a lifecycle with $1 in the setup_command.
        let mut state = AppState::default_with_scope_focus();
        let config = lifecycle_config("fossil branch", Some("script.sh $1"), None);

        // When handling SessionLifecycleSetup with args.
        let result = handle_session_lifecycle_setup(
            &mut state,
            "fossil branch",
            &["my-branch".to_owned()],
            None,
            &config,
        );

        // Then PersistSession is emitted first.
        assert!(result.message_names[0].contains("PersistSession"));
        // And RunSessionSetup is emitted third with rendered args.
        assert!(result.message_names[2].contains("RunSessionSetup"));
        // And the session has the args stored.
        assert_eq!(
            state.active_session().lifecycle_args(),
            &["my-branch".to_owned()]
        );
    }

    #[rstest::rstest]
    fn session_lifecycle_setup_clears_overlays_and_pushes_input() {
        // Given a state with a picker overlay.
        let mut state = AppState::default_with_scope_focus();
        state.frontend.scope_push(jinn_slices::FocusScope::Dynamic(
            jinn_project_msg::project_picker_scope(),
        ));

        // When handling SessionLifecycleSetup.
        let _result = handle_session_lifecycle_setup(
            &mut state,
            "",
            &[],
            None,
            crate::common::render_ctx::empty_config_layer(),
        );

        // Then overlays are cleared and Input scope is pushed.
        assert!(matches!(
            state.frontend.scope(),
            jinn_slices::FocusScope::Input
        ));
    }

    #[rstest::rstest]
    fn session_close_without_lifecycle_emits_close_session() {
        // Given a state with two sessions.
        let mut state = AppState::default_with_scope_focus();
        let second_session = ChatSessionState::new();
        let second_id = second_session.session_id().clone();
        state.session.insert(second_session);
        state.session.set_active(second_id);

        // When handling SessionClose.
        let result = handle_session_close(&mut state);

        // Then a CloseSession command is emitted for the closed session.
        assert_eq!(result.message_names.len(), 1);
        assert!(result.message_names[0].contains("CloseSession"));
    }

    #[rstest::rstest]
    fn session_close_with_teardown_emits_close_session() {
        // Given a session with a lifecycle that has a teardown_command.
        let mut state = AppState::default_with_scope_focus();
        let _config = lifecycle_config(
            "fossil branch",
            Some("echo /tmp/workdir"),
            Some("cleanup.sh $1"),
        );
        let session_id = state.session.active_session_id().clone();
        state
            .active_session_mut()
            .set_lifecycle_name(Some("fossil branch".to_owned()));
        state
            .active_session_mut()
            .set_lifecycle_args(vec!["my-branch".to_owned()]);

        // When handling SessionClose.
        let result = handle_session_close(&mut state);

        // Then a CloseSession command is emitted (actor handles teardown).
        assert!(state.session.contains(&session_id));
        assert_eq!(result.message_names.len(), 1);
        assert!(result.message_names[0].contains("CloseSession"));
    }

    #[rstest::rstest]
    fn session_close_last_session_emits_close_session() {
        // Given a state with only one session.
        let mut state = AppState::default_with_scope_focus();
        let _session_id = state.session.active_session_id().clone();
        assert_eq!(state.session.session_count(), 1);

        // When handling SessionClose.
        let result = handle_session_close(&mut state);

        // Then a CloseSession command is emitted.
        assert_eq!(result.message_names.len(), 1);
        assert!(result.message_names[0].contains("CloseSession"));
    }

    #[rstest::rstest]
    fn session_new_delegates_to_blank_lifecycle() {
        // Given default state.
        let mut state = AppState::default_with_scope_focus();
        let old_id = state.session.active_session_id().clone();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("old"));

        // When handling SessionNew (delegates to blank lifecycle setup).
        let result = crate::feat::session::intent::handle_session_new(
            &mut state,
            crate::common::render_ctx::empty_config_layer(),
        );

        // Then a new session is created (same behavior as before).
        assert_ne!(*state.session.active_session_id(), old_id);
        // And one message emitted (SessionCreated).
        assert_eq!(result.message_names.len(), 1);
        assert!(result.message_names[0].contains("SessionCreated"));
    }

    #[rstest::rstest]
    fn empty_session_is_preserved_on_new_session() {
        // Given default state with a single empty session.
        let mut state = AppState::default_with_scope_focus();
        let old_id = state.session.active_session_id().clone();
        assert!(state.active_session().is_empty());

        // When creating a new session via lifecycle setup.
        let _result = handle_session_lifecycle_setup(
            &mut state,
            "",
            &[],
            None,
            crate::common::render_ctx::empty_config_layer(),
        );

        // Then the old empty session is preserved (no auto-close).
        assert!(state.session.contains(&old_id));
        // And two sessions exist.
        assert_eq!(state.session.session_count(), 2);
    }

    #[rstest::rstest]
    fn session_with_history_is_preserved_on_new_session() {
        // Given an active session with history.
        let mut state = AppState::default_with_scope_focus();
        let old_id = state.session.active_session_id().clone();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));

        // When creating a new session.
        let _result = handle_session_lifecycle_setup(
            &mut state,
            "",
            &[],
            None,
            crate::common::render_ctx::empty_config_layer(),
        );

        // Then the old session is preserved.
        assert!(state.session.contains(&old_id));
        // And two sessions exist.
        assert_eq!(state.session.session_count(), 2);
        // And the new session is active.
        assert_ne!(*state.session.active_session_id(), old_id);
    }

    #[rstest::rstest]
    fn lifecycle_setup_seeds_reasoning_effort_from_global_default() {
        // Given a global default effort of High.
        let mut state = AppState::default_with_scope_focus();
        state.frontend.app_state.reasoning_effort = Some(crate::ReasoningEffort::High);

        // When creating a new session via lifecycle setup.
        let _result = handle_session_lifecycle_setup(
            &mut state,
            "",
            &[],
            None,
            crate::common::render_ctx::empty_config_layer(),
        );

        // Then the new session owns the seeded effort (a copy, not a live reference).
        assert_eq!(
            state.active_session().profile().reasoning_effort,
            Some(crate::ReasoningEffort::High),
            "new session should be seeded from the global default"
        );
    }

    #[rstest::rstest]
    fn lifecycle_setup_seeds_none_reasoning_effort_when_global_unset() {
        // Given no global default effort.
        let mut state = AppState::default_with_scope_focus();
        state.frontend.app_state.reasoning_effort = None;

        // When creating a new session via lifecycle setup.
        let _result = handle_session_lifecycle_setup(
            &mut state,
            "",
            &[],
            None,
            crate::common::render_ctx::empty_config_layer(),
        );

        // Then the new session's effort is None (provider decides).
        assert_eq!(
            state.active_session().profile().reasoning_effort,
            None,
            "new session should be seeded as None when global is unset"
        );
    }

    #[rstest::rstest]
    fn lifecycle_setup_seeds_disabled_tools_and_skills_from_config() {
        // Given configuration disabling a tool and a skill.
        let mut state = AppState::default_with_scope_focus();
        let config = jinn_config::testutil::config_layer(
            "[tools]\ndisabled = [\"bash\"]\n\
             [skills]\ndisabled = [\"phased-task-loop\"]\n",
        );

        // When creating a new session via lifecycle setup.
        let _result = handle_session_lifecycle_setup(&mut state, "", &[], None, &config);

        // Then the new session carries both disablement sets.
        assert!(state.active_session().disabled_tools().contains("bash"));
        // And the skill too.
        assert!(
            state
                .active_session()
                .disabled_skills()
                .contains("phased-task-loop")
        );
    }

    #[rstest::rstest]
    fn lifecycle_setup_with_auto_enabled_mcp_returns_enablement_message() {
        // Given configuration with one auto-enabled MCP server.
        let mut state = AppState::default_with_scope_focus();
        let config = jinn_config::testutil::config_layer(
            "[mcp.excalimate]\nauto_enable = true\ncommand = \"npx\"\n",
        );

        // When creating a new session (blank lifecycle).
        let result = handle_session_lifecycle_setup(&mut state, "", &[], None, &config);

        // Then the new session has the server enabled.
        assert!(state.active_session().is_mcp_server_enabled("excalimate"));
        // And an McpEnablementChanged message is emitted so the coordinator
        // spawns the connection.
        assert!(
            result
                .message_names
                .iter()
                .any(|n| n.contains("McpEnablementChanged"))
        );
    }

    #[rstest::rstest]
    fn lifecycle_setup_without_auto_enable_emits_no_enablement_message() {
        // Given configuration with a server that is NOT auto-enabled.
        let mut state = AppState::default_with_scope_focus();
        let config = jinn_config::testutil::config_layer(
            "[mcp.manual]\nauto_enable = false\ncommand = \"npx\"\n",
        );

        // When creating a new session.
        let result = handle_session_lifecycle_setup(&mut state, "", &[], None, &config);

        // Then the server is not enabled on the new session.
        assert!(!state.active_session().is_mcp_server_enabled("manual"));
        // And no enablement message is emitted.
        assert!(
            !result
                .message_names
                .iter()
                .any(|n| n.contains("McpEnablementChanged"))
        );
    }

    #[rstest::rstest]
    fn scripted_lifecycle_setup_with_auto_enable_attaches_enablement_message() {
        // Given a scripted lifecycle and one auto-enabled server.
        let mut state = AppState::default_with_scope_focus();
        let config = jinn_config::testutil::config_layer(
            "[[session_lifecycle.script]]\nname = \"fossil branch\"\n\
             setup_command = \"echo /tmp/workdir\"\n\
             [mcp.excalimate]\nauto_enable = true\ncommand = \"npx\"\n",
        );

        // When creating a session with the scripted lifecycle.
        let result =
            handle_session_lifecycle_setup(&mut state, "fossil branch", &[], None, &config);

        // Then the enablement message follows SessionCreated in the chain.
        let created_idx = result
            .message_names
            .iter()
            .position(|n| n.contains("SessionCreated"))
            .expect("SessionCreated emitted");
        let enablement_idx = result
            .message_names
            .iter()
            .position(|n| n.contains("McpEnablementChanged"))
            .expect("enablement emitted for scripted path");
        assert!(
            enablement_idx > created_idx,
            "enablement must trail SessionCreated"
        );
    }

    #[rstest::rstest]
    fn lifecycle_setup_preserves_empty_session_when_creating_lifecycle_session() {
        // Given a single empty session (app just started).
        let mut state = AppState::default_with_scope_focus();
        assert_eq!(state.session.session_count(), 1);

        // When creating a new session with a lifecycle.
        let config = lifecycle_config("fossil branch", Some("echo /tmp/workdir"), None);
        let result =
            handle_session_lifecycle_setup(&mut state, "fossil branch", &[], None, &config);

        // Then both sessions exist (old empty one is preserved).
        assert_eq!(state.session.session_count(), 2);
        // And the new session has the lifecycle name.
        assert_eq!(
            state.active_session().lifecycle_name(),
            Some("fossil branch")
        );
        // And PersistSession, PushChatEntry, then RunSessionSetup are emitted.
        assert!(result.message_names[0].contains("PersistSession"));
        assert!(result.message_names[1].contains("PushChatEntry"));
        assert!(result.message_names[2].contains("RunSessionSetup"));
    }

    #[rstest::rstest]
    #[test]
    fn abandon_via_enter_normal_mode_clears_pending_creation() {
        // Given a state with a pending session creation stashed from a
        // project-picker confirm (midway through the lifecycle/args chain).
        let mut state = AppState::default_with_scope_focus();
        let active_cwd = state.active_session().cwd().to_path_buf();
        state.frontend.pending_creation =
            Some(crate::feat::ui::frontend_state::PendingSessionCreation {
                project_dir: std::path::PathBuf::from("/tmp/project-a"),
                starting_cwd: std::path::PathBuf::from("/tmp/project-a"),
            });

        // When abandoning the chain via ESC (EnterNormalMode).
        let _result = crate::feat::chat_input::intent::handle_enter_normal_mode(
            &mut state,
            crate::common::render_ctx::empty_config_layer(),
        );

        // Then the stash is cleared so it never leaks into a future
        // `n`/`N`.
        assert!(state.frontend.pending_creation.is_none());
        // And the active session's CWD is unchanged (no side-channel mutation).
        assert_eq!(state.active_session().cwd(), active_cwd);
    }

    #[rstest::rstest]
    fn build_run_session_teardown_renders_command_with_args() {
        // Given a session with a lifecycle that has a teardown command.
        let mut state = AppState::default_with_scope_focus();
        let config = lifecycle_config("fossil branch", None, Some("cleanup.sh $1"));
        let session_id = state.session.active_session_id().clone();
        state
            .active_session_mut()
            .set_lifecycle_name(Some("fossil branch".to_owned()));
        state
            .active_session_mut()
            .set_lifecycle_args(vec!["my-branch".to_owned()]);

        // When building the teardown command.
        let msg = build_run_session_teardown(&state, &session_id, &config);

        // Then a rendered RunSessionTeardown is returned.
        let msg = msg.expect("teardown command should be built");
        assert_eq!(msg.session_id, session_id);
        assert_eq!(msg.command, "cleanup.sh my-branch");
        assert_eq!(msg.args, vec!["my-branch".to_owned()]);
    }

    #[rstest::rstest]
    fn build_run_session_teardown_returns_none_without_teardown_command() {
        // Given a session whose lifecycle has no teardown command.
        let mut state = AppState::default_with_scope_focus();
        let config = lifecycle_config("blank", None, None);
        let session_id = state.session.active_session_id().clone();
        state
            .active_session_mut()
            .set_lifecycle_name(Some("blank".to_owned()));

        // When building the teardown command.
        let msg = build_run_session_teardown(&state, &session_id, &config);

        // Then None is returned.
        assert!(msg.is_none());
    }

    #[rstest::rstest]
    fn build_run_session_teardown_returns_none_without_lifecycle_name() {
        // Given a session with no lifecycle name.
        let state = AppState::default_with_scope_focus();
        let session_id = state.session.active_session_id().clone();

        // When building the teardown command.
        let msg = build_run_session_teardown(
            &state,
            &session_id,
            crate::common::render_ctx::empty_config_layer(),
        );

        // Then None is returned.
        assert!(msg.is_none());
    }

    #[rstest::rstest]
    fn build_run_session_teardown_renders_positional_arg_from_stored_args() {
        // Given a lifecycle teardown with $1 and a session storing the arg.
        let mut state = AppState::default_with_scope_focus();
        let config = lifecycle_config("fossil branch", None, Some("cleanup.sh $1"));
        let session_id = state.session.active_session_id().clone();
        state
            .active_session_mut()
            .set_lifecycle_name(Some("fossil branch".to_owned()));
        state
            .active_session_mut()
            .set_lifecycle_args(vec!["feature-x".to_owned()]);

        // When building the teardown command.
        let msg = build_run_session_teardown(&state, &session_id, &config);

        // Then the $1 positional is rendered with the stored arg.
        let msg = msg.expect("teardown command should be built");
        assert_eq!(msg.command, "cleanup.sh feature-x");
    }
}
