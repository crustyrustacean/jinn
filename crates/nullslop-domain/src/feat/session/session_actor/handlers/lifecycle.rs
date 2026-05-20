//! Session lifecycle handlers — manage setup, teardown, close, and archive operations.
//!
//! Handles the full session lifecycle: running setup/teardown commands, closing sessions
//! (with optional teardown), archiving sessions to SQLite, and creating replacement sessions
//! when the last one is removed. Also contains the helper functions for building lifecycle
//! chat entries and formatting command errors.

use crate::common::actor::ActorContext;
use crate::feat::chat_input::protocol::command::PushChatEntry;
use crate::feat::chat_input::protocol::event::ChatEntrySubmitted;
use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::session::protocol::archive_session::ArchiveSession;
use crate::feat::session::protocol::close_session::CloseSession;
use crate::feat::session::protocol::session_archived::SessionArchived;
use crate::feat::session::protocol::session_closed::SessionClosed;
use crate::feat::session_lifecycle::command_runner::LifecycleCommandError;
use crate::feat::session_lifecycle::command_runner::run_setup_command;
use crate::feat::session_lifecycle::command_runner::run_teardown_command;
use crate::feat::session_lifecycle::protocol::command::{
    RunSessionSetup, RunSessionTeardown, SaveNewLifecycleSession,
};
use crate::feat::session_lifecycle::protocol::event::{
    SessionSetupCompleted, SessionTeardownFinished,
};
use crate::protocol::{ChatEntry, Command, Event, PromptStrategyId};

use super::super::SessionPersistenceActor;

/// Remove ANSI escape sequences from a string.
pub(in crate::feat::session::session_actor) fn strip_ansi(s: &str) -> String {
    strip_ansi_escapes::strip_str(s)
}

/// Build a system chat entry for when a setup command produces no output.
///
/// Shows the fallback CWD message.
pub(in crate::feat::session::session_actor) fn no_output_info(
    default_cwd: &std::path::Path,
) -> ChatEntry {
    ChatEntry::system(format!(
        "No path returned by setup command. Using {} as cwd",
        default_cwd.display()
    ))
}

/// System entry shown while a setup command is running.
pub(crate) fn setup_running_msg() -> ChatEntry {
    ChatEntry::system("⚙️ Running setup script...")
}

/// System entry shown when a setup command completes successfully.
pub(in crate::feat::session::session_actor) fn setup_complete_msg(
    cwd: &std::path::Path,
) -> ChatEntry {
    ChatEntry::system(format!(
        "✅ Setup complete — Using {} as cwd",
        cwd.display()
    ))
}

/// System entry shown while a teardown command is running.
pub(in crate::feat::session::session_actor) fn teardown_running_msg() -> ChatEntry {
    ChatEntry::system("⚙️ Running teardown script...")
}

/// System entry shown when a teardown-only command succeeds (session is kept open).
pub(in crate::feat::session::session_actor) fn teardown_success_msg() -> ChatEntry {
    ChatEntry::system("✅ Teardown completed successfully.")
}

/// Format a `LifecycleCommandError` into a clean user-facing message.
pub(in crate::feat::session::session_actor) fn format_lifecycle_error(
    err: &LifecycleCommandError,
) -> String {
    match err {
        LifecycleCommandError::CommandFailed {
            exit_code,
            stdout,
            stderr,
        } => {
            let mut parts = vec![format!("Command failed (exit code: {:?})", exit_code)];
            if !stdout.is_empty() {
                parts.push(format!("stdout:\n{}", strip_ansi(stdout)));
            }
            if !stderr.is_empty() {
                parts.push(format!("stderr:\n{}", strip_ansi(stderr)));
            }
            parts.join("\n\n")
        }
        LifecycleCommandError::NoOutput => "Command produced no output".to_owned(),
        LifecycleCommandError::InvalidPath { path } => {
            format!(
                "Path does not exist or cannot be resolved: {}",
                path.display()
            )
        }
        LifecycleCommandError::NotADirectory { path } => {
            format!("Path is not a directory: {}", path.display())
        }
        LifecycleCommandError::ExecutionFailed => "Failed to execute command".to_owned(),
    }
}

impl SessionPersistenceActor {
    /// Push a chat entry directly and save.
    ///
    /// For use when the push must happen before an `.await` (e.g., teardown
    /// "running" message). Otherwise, prefer emitting a `PushChatEntry` command
    /// which routes through `handle_push_chat_entry` and persists automatically.
    pub(in crate::feat::session::session_actor) async fn push_and_save(
        &self,
        session_id: &crate::protocol::SessionId,
        entry: ChatEntry,
        ctx: &ActorContext,
    ) {
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(session_id);
            session.push_entry(entry.clone());
        }
        if let Err(e) = ctx.send_event(Event::ChatEntrySubmitted(ChatEntrySubmitted {
            session_id: session_id.clone(),
            entry,
        })) {
            tracing::warn!(err = ?e, "session-actor failed to emit ChatEntrySubmitted");
        }
        self.save_active_session(session_id).await;
    }

    /// RunSessionSetup: execute the lifecycle setup command asynchronously.
    ///
    /// On success, sets the session's CWD to the command's output.
    /// On failure, sets the default CWD and pushes an error entry.
    pub(in crate::feat::session::session_actor) async fn handle_run_session_setup(
        &self,
        payload: &RunSessionSetup,
        ctx: &ActorContext,
    ) {
        // "running" entry is now emitted by the intent handler as a PushChatEntry
        // command before this handler runs. No direct push needed here.

        let result = run_setup_command(&payload.command).await;

        match result {
            Ok(cwd) => {
                {
                    let mut state = self.state.write();
                    if let Some(session) = state.session.sessions_mut().get_mut(&payload.session_id)
                    {
                        session.set_cwd(cwd.clone());
                        session.advance_lifecycle_after_setup();
                    }
                }

                // Emit the completion entry via PushChatEntry (persists automatically).
                if let Err(e) = ctx.send_command(Command::PushChatEntry(PushChatEntry {
                    session_id: payload.session_id.clone(),
                    entry: setup_complete_msg(&cwd),
                })) {
                    tracing::warn!(err = ?e, "session-actor failed to emit PushChatEntry for setup complete");
                }

                if let Err(e) =
                    ctx.send_event(Event::SessionSetupCompleted(SessionSetupCompleted {
                        session_id: payload.session_id.clone(),
                        cwd,
                        error: None,
                    }))
                {
                    tracing::warn!(err = ?e, "session-actor failed to emit SessionSetupCompleted");
                }
            }
            Err(report) => {
                let is_no_output = matches!(
                    report.downcast_ref::<LifecycleCommandError>(),
                    Some(LifecycleCommandError::NoOutput)
                );
                let error_msg =
                    if let Some(cmd_err) = report.downcast_ref::<LifecycleCommandError>() {
                        format_lifecycle_error(cmd_err)
                    } else {
                        strip_ansi(&format!("{report:#?}"))
                    };
                let default_cwd = {
                    let mut state = self.state.write();
                    let default = state.session.default_cwd().clone();
                    if let Some(session) = state.session.sessions_mut().get_mut(&payload.session_id)
                    {
                        session.set_cwd(default.clone());
                    }
                    default
                };

                let entry = if is_no_output {
                    no_output_info(&default_cwd)
                } else {
                    ChatEntry::error(&error_msg)
                };

                // Emit the error entry via PushChatEntry (persists automatically).
                if let Err(e) = ctx.send_command(Command::PushChatEntry(PushChatEntry {
                    session_id: payload.session_id.clone(),
                    entry,
                })) {
                    tracing::warn!(err = ?e, "session-actor failed to emit PushChatEntry for setup error");
                }

                if let Err(e) =
                    ctx.send_event(Event::SessionSetupCompleted(SessionSetupCompleted {
                        session_id: payload.session_id.clone(),
                        cwd: default_cwd,
                        error: Some(error_msg),
                    }))
                {
                    tracing::warn!(err = ?e, "session-actor failed to emit SessionSetupCompleted");
                }
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    pub(in crate::feat::session::session_actor) async fn handle_run_session_teardown(
        &self,
        payload: &RunSessionTeardown,
        ctx: &ActorContext,
    ) {
        // Push "running" entry directly and save (before .await).
        self.push_and_save(&payload.session_id, teardown_running_msg(), ctx)
            .await;

        let result = run_teardown_command(&payload.command).await;

        match result {
            Ok(()) => {
                if payload.close_on_success {
                    // Teardown succeeded - remove session and switch active.
                    {
                        let mut state = self.state.write();
                        state.session.sessions_mut().remove(&payload.session_id);
                        if state.session.sessions().is_empty() {
                            let model = state
                                .frontend
                                .preferences
                                .last_model
                                .clone()
                                .unwrap_or_else(|| {
                                    crate::feat::provider_infra::NO_PROVIDER_ID.to_owned()
                                });
                            let strategy = state
                                .frontend
                                .preferences
                                .last_strategy
                                .as_deref()
                                .map_or_else(
                                    crate::protocol::PromptStrategyId::passthrough,
                                    crate::protocol::PromptStrategyId::new,
                                );
                            let token_budget =
                                state.frontend.preferences.context_token_budget.budget;
                            let sliding_window_size =
                                state.frontend.preferences.context_sliding_window.size;
                            let new_session = ChatSessionState::new_with_profile(
                                crate::feat::session::profile::SessionProfile::from_config(
                                    model,
                                    strategy,
                                    token_budget,
                                    sliding_window_size,
                                ),
                            );
                            let new_id = new_session.session_id().clone();
                            state
                                .session
                                .sessions_mut()
                                .insert(new_id.clone(), new_session);
                            state.session.set_active(new_id);
                        } else if *state.session.active_session_id() == payload.session_id {
                            let next_id = state
                                .session
                                .sessions()
                                .keys()
                                .next()
                                .expect("sessions is non-empty")
                                .clone();
                            state.session.set_active(next_id);
                        }
                    }

                    if let Err(e) =
                        ctx.send_event(Event::SessionTeardownFinished(SessionTeardownFinished {
                            session_id: payload.session_id.clone(),
                            error: None,
                        }))
                    {
                        tracing::warn!(err = ?e, "session-actor failed to emit SessionTeardownFinished");
                    }

                    if let Err(e) = ctx.send_event(Event::SessionClosed(SessionClosed {
                        session_id: payload.session_id.clone(),
                    })) {
                        tracing::warn!(err = ?e, "session-actor failed to emit SessionClosed");
                    }
                } else {
                    // Teardown-only success — emit entry via PushChatEntry (persists automatically).
                    if let Err(e) = ctx.send_command(Command::PushChatEntry(PushChatEntry {
                        session_id: payload.session_id.clone(),
                        entry: teardown_success_msg(),
                    })) {
                        tracing::warn!(err = ?e, "session-actor failed to emit PushChatEntry for teardown success");
                    }

                    if let Err(e) =
                        ctx.send_event(Event::SessionTeardownFinished(SessionTeardownFinished {
                            session_id: payload.session_id.clone(),
                            error: None,
                        }))
                    {
                        tracing::warn!(err = ?e, "session-actor failed to emit SessionTeardownFinished");
                    }
                }
            }
            Err(report) => {
                let error_msg =
                    if let Some(cmd_err) = report.downcast_ref::<LifecycleCommandError>() {
                        format_lifecycle_error(cmd_err)
                    } else {
                        strip_ansi(&format!("{report:#?}"))
                    };

                // Emit error entry via PushChatEntry (persists automatically).
                // Remove the old direct save — the command handler saves.
                if let Err(e) = ctx.send_command(Command::PushChatEntry(PushChatEntry {
                    session_id: payload.session_id.clone(),
                    entry: ChatEntry::error(&error_msg),
                })) {
                    tracing::warn!(err = ?e, "session-actor failed to emit PushChatEntry for teardown error");
                }

                if let Err(e) =
                    ctx.send_event(Event::SessionTeardownFinished(SessionTeardownFinished {
                        session_id: payload.session_id.clone(),
                        error: Some(error_msg),
                    }))
                {
                    tracing::warn!(err = ?e, "session-actor failed to emit SessionTeardownFinished");
                }
            }
        }
    }

    /// CloseSession: state-driven close handler.
    ///
    /// Uses `LifecycleScriptState` to decide whether to run teardown:
    /// - `SetupRan` → run teardown, archive, remove from memory
    /// - `NothingRan` or `TeardownRan` → archive, remove from memory (no teardown)
    ///
    /// Steps execute sequentially. On failure, the chain stops and the session
    /// remains in its current state.
    pub(in crate::feat::session::session_actor) async fn handle_close_session(
        &self,
        payload: &CloseSession,
        ctx: &ActorContext,
    ) {
        use crate::feat::session::chat_session::LifecycleScriptState;

        // Collect session info under read lock.
        let session_info = {
            let state = self.state.read();
            let Some(session) = state.session.sessions().get(&payload.session_id) else {
                return;
            };
            let script_state = session.lifecycle_script_state();
            let lifecycle_name = session.lifecycle_name().map(str::to_owned);
            let lifecycle_args = session.lifecycle_args().to_vec();
            (script_state, lifecycle_name, lifecycle_args)
        };

        let (script_state, lifecycle_name, lifecycle_args) = session_info;

        // Step 1: Run teardown if LifecycleScriptState is SetupRan.
        if script_state == LifecycleScriptState::SetupRan {
            let teardown_cmd = lifecycle_name.as_deref().and_then(|name| {
                let state = self.state.read();
                state
                    .frontend
                    .preferences
                    .session_lifecycles
                    .iter()
                    .find(|l| l.name == name)
                    .and_then(|l| l.teardown_command.clone())
            });

            if let Some(teardown_cmd) = teardown_cmd {
                let success = self
                    .run_close_teardown_step(
                        &payload.session_id,
                        &teardown_cmd,
                        &lifecycle_args,
                        ctx,
                    )
                    .await;

                if !success {
                    // Teardown failed — error already pushed, chain stops.
                    if let Err(e) =
                        ctx.send_event(Event::SessionTeardownFinished(SessionTeardownFinished {
                            session_id: payload.session_id.clone(),
                            error: Some("teardown failed".to_owned()),
                        }))
                    {
                        tracing::warn!(err = ?e, "session-actor failed to emit SessionTeardownFinished");
                    }
                    return;
                }

                // Advance LifecycleScriptState: SetupRan → TeardownRan.
                {
                    let mut state = self.state.write();
                    if let Some(session) = state.session.sessions_mut().get_mut(&payload.session_id)
                    {
                        session.advance_lifecycle_after_teardown();
                    }
                }

                if let Err(e) =
                    ctx.send_event(Event::SessionTeardownFinished(SessionTeardownFinished {
                        session_id: payload.session_id.clone(),
                        error: None,
                    }))
                {
                    tracing::warn!(err = ?e, "session-actor failed to emit SessionTeardownFinished");
                }
            }
            // If no teardown command exists but state is SetupRan, skip teardown and proceed to archive.
        }

        // Step 2: Archive in DB.
        if let Some(ref store) = self.store
            && let Err(e) = store.set_archived(&payload.session_id, true).await
        {
            tracing::warn!(err = ?e, "failed to archive session");
        }

        // Step 3: Remove from memory.
        self.close_session_inline(&payload.session_id, ctx);

        if let Err(e) = ctx.send_event(Event::SessionArchived(SessionArchived {
            session_id: payload.session_id.clone(),
        })) {
            tracing::warn!(err = ?e, "session-actor failed to emit SessionArchived");
        }
    }

    /// ArchiveSession: archive without running teardown.
    ///
    /// - Empty session → remove from map, no archive.
    /// - Non-empty → archive in SQLite, remove from map, emit events.
    ///
    /// Does NOT check or advance `LifecycleScriptState`. The session is simply
    /// put away — it can be unarchived later.
    pub(in crate::feat::session::session_actor) async fn handle_archive_session(
        &self,
        payload: &ArchiveSession,
        ctx: &ActorContext,
    ) {
        // Archive in SQLite.
        if let Some(ref store) = self.store
            && let Err(e) = store.set_archived(&payload.session_id, true).await
        {
            tracing::warn!(err = ?e, "failed to archive session");
        }

        self.close_session_inline(&payload.session_id, ctx);

        if let Err(e) = ctx.send_event(Event::SessionArchived(SessionArchived {
            session_id: payload.session_id.clone(),
        })) {
            tracing::warn!(err = ?e, "session-actor failed to emit SessionArchived");
        }
    }

    /// Runs the teardown command as part of session close.
    ///
    /// Returns `true` on success, `false` on failure (error entry pushed to session).
    pub(in crate::feat::session::session_actor) async fn run_close_teardown_step(
        &self,
        session_id: &crate::protocol::SessionId,
        teardown_cmd: &str,
        lifecycle_args: &[String],
        ctx: &ActorContext,
    ) -> bool {
        use crate::feat::session_lifecycle::command_template::CommandTemplate;

        // Push "running" entry directly and save (before .await).
        self.push_and_save(session_id, teardown_running_msg(), ctx)
            .await;

        let template = CommandTemplate::parse(teardown_cmd);
        let rendered = if lifecycle_args.is_empty() {
            teardown_cmd.to_owned()
        } else {
            template.render(lifecycle_args)
        };

        match run_teardown_command(&rendered).await {
            Ok(()) => true,
            Err(report) => {
                let error_msg =
                    if let Some(cmd_err) = report.downcast_ref::<LifecycleCommandError>() {
                        format_lifecycle_error(cmd_err)
                    } else {
                        strip_ansi(&format!("{report:#?}"))
                    };

                // Emit error entry via PushChatEntry (persists automatically).
                if let Err(e) = ctx.send_command(Command::PushChatEntry(PushChatEntry {
                    session_id: session_id.clone(),
                    entry: ChatEntry::error(&error_msg),
                })) {
                    tracing::warn!(err = ?e, "session-actor failed to emit PushChatEntry for teardown error");
                }
                false
            }
        }
    }

    /// Remove session from HashMap, create new if empty, switch active,
    /// adjust sidebar cursor, and emit `SessionClosed`.
    pub(in crate::feat::session::session_actor) fn close_session_inline(
        &self,
        session_id: &crate::protocol::SessionId,
        ctx: &ActorContext,
    ) {
        {
            let mut state = self.state.write();
            state.session.sessions_mut().remove(session_id);

            if state.session.sessions().is_empty() {
                let model = state
                    .frontend
                    .preferences
                    .last_model
                    .clone()
                    .unwrap_or_else(|| crate::feat::provider_infra::NO_PROVIDER_ID.to_owned());
                let strategy = state
                    .frontend
                    .preferences
                    .last_strategy
                    .as_deref()
                    .map_or_else(PromptStrategyId::passthrough, PromptStrategyId::new);
                let token_budget = state.frontend.preferences.context_token_budget.budget;
                let sliding_window_size = state.frontend.preferences.context_sliding_window.size;
                let new_session = ChatSessionState::new_with_profile(
                    crate::feat::session::profile::SessionProfile::from_config(
                        model,
                        strategy,
                        token_budget,
                        sliding_window_size,
                    ),
                );
                let new_id = new_session.session_id().clone();
                state
                    .session
                    .sessions_mut()
                    .insert(new_id.clone(), new_session);
                state.session.set_active(new_id);
            } else if *state.session.active_session_id() == *session_id {
                let next_id = state
                    .session
                    .sessions()
                    .keys()
                    .next()
                    .expect("sessions is non-empty")
                    .clone();
                state.session.set_active(next_id);
            }
        }

        if let Err(e) = ctx.send_event(Event::SessionClosed(SessionClosed {
            session_id: session_id.clone(),
        })) {
            tracing::warn!(err = ?e, "session-actor failed to emit SessionClosed");
        }
    }

    /// SaveNewLifecycleSession: persist the session immediately.
    ///
    /// Called right after the IntentHandler creates a lifecycle session
    /// so that the session metadata survives an app crash during setup.
    pub(in crate::feat::session::session_actor) async fn handle_save_new_lifecycle_session(
        &self,
        payload: &SaveNewLifecycleSession,
    ) {
        self.save_active_session(&payload.session_id).await;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::super::super::helpers::{test_actor, test_context};
    use super::{
        no_output_info, setup_complete_msg, setup_running_msg, strip_ansi, teardown_running_msg,
    };
    use crate::feat::chat_input::protocol::command::PushChatEntry;
    use crate::feat::session::chat_session::ChatSessionState;
    use crate::protocol::{ChatEntry, ChatEntryKind, Command};
    use std::path::Path;

    // --- Helper function tests ---

    #[rstest::rstest]
    fn strip_ansi_removes_bold_codes() {
        // Given text with bold ANSI codes.
        let input = "\x1b[1mbold text\x1b[22m";

        // When stripping ANSI.
        let result = strip_ansi(input);

        // Then the ANSI codes are removed.
        assert_eq!(result, "bold text");
    }

    #[rstest::rstest]
    fn strip_ansi_removes_color_codes() {
        let input = "\x1b[31mred\x1b[0m";
        let result = strip_ansi(input);
        assert_eq!(result, "red");
    }

    #[rstest::rstest]
    fn strip_ansi_passes_plain_text() {
        let input = "hello world";
        let result = strip_ansi(input);
        assert_eq!(result, "hello world");
    }

    #[rstest::rstest]
    fn strip_ansi_handles_chained_codes() {
        let input = "\x1b[1m\x1b[31mbold red\x1b[0m";
        let result = strip_ansi(input);
        assert_eq!(result, "bold red");
    }

    #[rstest::rstest]
    fn strip_ansi_handles_complex_csi_sequences() {
        // 38;5;196 is foreground 256-color (bright red).
        let input = "\x1b[38;5;196mcolored\x1b[0m";
        let result = strip_ansi(input);
        assert_eq!(result, "colored");
    }

    #[rstest::rstest]
    fn strip_ansi_handles_empty_string() {
        let result = strip_ansi("");
        assert_eq!(result, "");
    }

    #[rstest::rstest]
    fn strip_ansi_handles_text_with_no_ansi() {
        let input = "normal text\nwith newlines";
        let result = strip_ansi(input);
        assert_eq!(result, "normal text\nwith newlines");
    }

    #[rstest::rstest]
    fn no_output_info_is_system_entry_with_cwd() {
        // Given a default CWD path.
        let cwd = Path::new("/tmp/test-project");

        // When building the no-output info entry.
        let entry = no_output_info(cwd);

        // Then it is a System entry mentioning the CWD.
        let ChatEntryKind::System(text) = &entry.kind else {
            panic!("expected System entry, got {:?}", entry.kind);
        };
        assert!(text.contains("No path returned by setup command"));
        assert!(text.contains("/tmp/test-project"));
    }

    #[rstest::rstest]
    fn setup_running_msg_is_system_with_gear_emoji() {
        // When building the setup running message.
        let entry = setup_running_msg();

        // Then it is a System entry with gear emoji and running text.
        let ChatEntryKind::System(text) = &entry.kind else {
            panic!("expected System entry, got {:?}", entry.kind);
        };
        assert!(text.contains("⚙️"));
        assert!(text.contains("Running setup script"));
    }

    #[rstest::rstest]
    fn setup_complete_msg_is_system_with_checkmark_and_cwd() {
        // Given a CWD path.
        let cwd = Path::new("/tmp/my-project");

        // When building the setup complete message.
        let entry = setup_complete_msg(cwd);

        // Then it is a System entry with checkmark and CWD.
        let ChatEntryKind::System(text) = &entry.kind else {
            panic!("expected System entry, got {:?}", entry.kind);
        };
        assert!(text.contains("✅"));
        assert!(text.contains("Setup complete"));
        assert!(text.contains("/tmp/my-project"));
    }

    #[rstest::rstest]
    fn teardown_running_msg_is_system_with_gear_emoji() {
        // When building the teardown running message.
        let entry = teardown_running_msg();

        // Then it is a System entry with gear emoji and running text.
        let ChatEntryKind::System(text) = &entry.kind else {
            panic!("expected System entry, got {:?}", entry.kind);
        };
        assert!(text.contains("⚙️"));
        assert!(text.contains("Running teardown script"));
    }

    // --- Teardown failure handler tests ---

    #[tokio::test]
    async fn teardown_failure_does_not_switch_active_session() {
        // Given a session actor with two sessions, second is targeted for teardown.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let (target_id, original_active) = {
            let mut state = actor.state.write();
            let original_active = state.session.active_session_id().clone();
            let second = ChatSessionState::new();
            let second_id = second.session_id().clone();
            state
                .session
                .sessions_mut()
                .insert(second_id.clone(), second);
            (second_id, original_active)
        };

        // When handling a teardown command that fails targeting the non-active session.
        actor
            .handle_run_session_teardown(
                &crate::feat::session_lifecycle::protocol::command::RunSessionTeardown {
                    session_id: target_id,
                    command: "exit 1".to_owned(),
                    args: vec![],
                    close_on_success: true,
                },
                &ctx,
            )
            .await;

        // Then the active session is unchanged.
        let state = actor.state.read();
        assert_eq!(*state.session.active_session_id(), original_active);
        drop(state);

        // And SessionTeardownFinished was emitted with error.
        let events = sink.events();
        let teardown_evt = events
            .iter()
            .find(|e| matches!(e, crate::protocol::Event::SessionTeardownFinished(..)));
        assert!(
            teardown_evt.is_some(),
            "expected SessionTeardownFinished event"
        );
    }

    #[tokio::test]
    async fn teardown_failure_does_not_push_input_scope() {
        // Given a session actor in Normal scope.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let state = actor.state.read();
            state.session.active_session_id().clone()
        };

        // When handling a teardown command that fails.
        actor
            .handle_run_session_teardown(
                &crate::feat::session_lifecycle::protocol::command::RunSessionTeardown {
                    session_id,
                    command: "exit 1".to_owned(),
                    args: vec![],
                    close_on_success: true,
                },
                &ctx,
            )
            .await;

        // Then the scope stack does not have Input on it.
        let state = actor.state.read();
        assert!(!matches!(
            state.frontend.scope_stack.current(),
            crate::common::app_state::FocusScope::Input
        ));
    }

    #[tokio::test]
    async fn teardown_failure_pushes_error_entry_to_session() {
        // Given a session actor with a session.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let state = actor.state.read();
            state.session.active_session_id().clone()
        };

        // When handling a teardown command that fails.
        actor
            .handle_run_session_teardown(
                &crate::feat::session_lifecycle::protocol::command::RunSessionTeardown {
                    session_id: session_id.clone(),
                    command: "exit 1".to_owned(),
                    args: vec![],
                    close_on_success: true,
                },
                &ctx,
            )
            .await;

        // Then a PushChatEntry command with an error entry was emitted.
        let commands = sink.commands();
        let has_error = commands.iter().any(|cmd| {
            matches!(
                cmd,
                Command::PushChatEntry(PushChatEntry { entry, .. })
                if matches!(&entry.kind, ChatEntryKind::Error(msg) if msg.contains("exit code"))
            )
        });
        assert!(has_error, "expected PushChatEntry command with error entry");
    }

    #[tokio::test]
    async fn teardown_failure_emits_session_teardown_completed_with_error() {
        // Given a session actor with a session.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let state = actor.state.read();
            state.session.active_session_id().clone()
        };

        // When handling a teardown command that fails.
        actor
            .handle_run_session_teardown(
                &crate::feat::session_lifecycle::protocol::command::RunSessionTeardown {
                    session_id: session_id.clone(),
                    command: "exit 1".to_owned(),
                    args: vec![],
                    close_on_success: true,
                },
                &ctx,
            )
            .await;

        // Then SessionTeardownFinished is emitted with an error message.
        let events = sink.events();
        let found = events.iter().any(|e| {
            matches!(
                e,
                crate::protocol::Event::SessionTeardownFinished(
                    crate::feat::session_lifecycle::protocol::event::SessionTeardownFinished {
                        error: Some(..),
                        session_id: sid,
                    }
                ) if sid == &session_id
            )
        });
        assert!(found, "expected SessionTeardownFinished with error");
    }

    // --- CloseSession handler tests ---

    #[tokio::test]
    async fn remove_session_removes_session_from_hashmap() {
        // Given a session actor with two sessions.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let second = ChatSessionState::new();
        let second_id = second.session_id().clone();
        {
            let mut state = actor.state.write();
            state
                .session
                .sessions_mut()
                .insert(second_id.clone(), second);
        }

        // When handling CloseSession for the second session.
        actor
            .handle_close_session(
                &crate::feat::session::protocol::close_session::CloseSession {
                    session_id: second_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then the second session is removed.
        let state = actor.state.read();
        assert!(!state.session.sessions().contains_key(&second_id));
        // And the first session still exists.
        assert_eq!(state.session.sessions().len(), 1);
        drop(state);

        // And SessionClosed is emitted.
        let events = sink.events();
        let found = events.iter().any(|e| {
            matches!(
                e,
                crate::protocol::Event::SessionClosed(
                    crate::feat::session::protocol::session_closed::SessionClosed {
                        session_id: sid,
                    }
                ) if sid == &second_id
            )
        });
        assert!(found, "expected SessionClosed event");
    }

    #[tokio::test]
    async fn remove_session_creates_new_session_when_last_removed() {
        // Given a session actor with only one session.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let only_id = {
            let state = actor.state.read();
            state.session.active_session_id().clone()
        };

        // When handling CloseSession for the only session.
        actor
            .handle_close_session(
                &crate::feat::session::protocol::close_session::CloseSession {
                    session_id: only_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then the only session is removed and a new one is created.
        let state = actor.state.read();
        assert!(!state.session.sessions().contains_key(&only_id));
        assert_eq!(state.session.sessions().len(), 1);
        assert_ne!(*state.session.active_session_id(), only_id);
        drop(state);

        // And SessionClosed is emitted.
        let events = sink.events();
        let found = events.iter().any(|e| {
            matches!(
                e,
                crate::protocol::Event::SessionClosed(
                    crate::feat::session::protocol::session_closed::SessionClosed {
                        session_id: sid,
                    }
                ) if sid == &only_id
            )
        });
        assert!(found, "expected SessionClosed event");
    }

    #[tokio::test]
    async fn remove_session_switches_active_when_active_is_removed() {
        // Given a session actor with two sessions, active is the second.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let second = ChatSessionState::new();
        let second_id = second.session_id().clone();
        {
            let mut state = actor.state.write();
            state
                .session
                .sessions_mut()
                .insert(second_id.clone(), second);
            state.session.set_active(second_id.clone());
        }

        // When handling CloseSession for the active session.
        actor
            .handle_close_session(
                &crate::feat::session::protocol::close_session::CloseSession {
                    session_id: second_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then active session is switched to the remaining one.
        let state = actor.state.read();
        assert_ne!(*state.session.active_session_id(), second_id);
        assert_eq!(state.session.sessions().len(), 1);
    }

    #[tokio::test]
    async fn remove_session_emits_session_removed_event() {
        // Given a session actor with two sessions.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let second = ChatSessionState::new();
        let second_id = second.session_id().clone();
        {
            let mut state = actor.state.write();
            state
                .session
                .sessions_mut()
                .insert(second_id.clone(), second);
        }

        // When handling CloseSession.
        actor
            .handle_close_session(
                &crate::feat::session::protocol::close_session::CloseSession {
                    session_id: second_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then exactly one SessionClosed event is emitted for the correct session.
        let events = sink.events();
        let count = events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    crate::protocol::Event::SessionClosed(
                        crate::feat::session::protocol::session_closed::SessionClosed {
                            session_id: sid,
                        }
                    ) if sid == &second_id
                )
            })
            .count();
        assert_eq!(count, 1, "expected exactly one SessionClosed event");
    }

    #[tokio::test]
    async fn remove_session_is_noop_if_session_does_not_exist() {
        // Given a session actor with one session.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let fake_id = crate::protocol::SessionId::new();
        let original_len = {
            let state = actor.state.read();
            state.session.sessions().len()
        };

        // When handling CloseSession for a nonexistent session.
        actor
            .handle_close_session(
                &crate::feat::session::protocol::close_session::CloseSession {
                    session_id: fake_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then nothing changes.
        let state = actor.state.read();
        assert_eq!(state.session.sessions().len(), original_len);
        drop(state);

        // And no SessionClosed event is emitted.
        let events = sink.events();
        let found = events.iter().any(|e| {
            matches!(
                e,
                crate::protocol::Event::SessionClosed(
                    crate::feat::session::protocol::session_closed::SessionClosed {
                        session_id: sid,
                    }
                ) if sid == &fake_id
            )
        });
        assert!(
            !found,
            "did not expect SessionClosed for nonexistent session"
        );
    }

    // --- Teardown-only (close_on_success: false) tests ---

    #[tokio::test]
    async fn teardown_only_success_does_not_remove_session() {
        // Given a session actor with a session.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let state = actor.state.read();
            state.session.active_session_id().clone()
        };
        let original_count = {
            let state = actor.state.read();
            state.session.sessions().len()
        };

        // When teardown succeeds with close_on_success: false.
        actor
            .handle_run_session_teardown(
                &crate::feat::session_lifecycle::protocol::command::RunSessionTeardown {
                    session_id: session_id.clone(),
                    command: "echo test".to_owned(),
                    args: vec![],
                    close_on_success: false,
                },
                &ctx,
            )
            .await;

        // Then the session is NOT removed.
        let state = actor.state.read();
        assert!(state.session.sessions().contains_key(&session_id));
        assert_eq!(state.session.sessions().len(), original_count);
    }

    #[tokio::test]
    async fn teardown_only_success_does_not_emit_session_removed() {
        // Given a session actor with a session.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let state = actor.state.read();
            state.session.active_session_id().clone()
        };

        // When teardown succeeds with close_on_success: false.
        actor
            .handle_run_session_teardown(
                &crate::feat::session_lifecycle::protocol::command::RunSessionTeardown {
                    session_id: session_id.clone(),
                    command: "echo test".to_owned(),
                    args: vec![],
                    close_on_success: false,
                },
                &ctx,
            )
            .await;

        // Then no SessionClosed event is emitted.
        let events = sink.events();
        let found = events.iter().any(|e| {
            matches!(
                e,
                crate::protocol::Event::SessionClosed(
                    crate::feat::session::protocol::session_closed::SessionClosed {
                        session_id: sid,
                    }
                ) if sid == &session_id
            )
        });
        assert!(!found, "did not expect SessionClosed for teardown-only");
    }

    #[tokio::test]
    async fn teardown_only_success_emits_session_teardown_completed() {
        // Given a session actor with a session.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let state = actor.state.read();
            state.session.active_session_id().clone()
        };

        // When teardown succeeds with close_on_success: false.
        actor
            .handle_run_session_teardown(
                &crate::feat::session_lifecycle::protocol::command::RunSessionTeardown {
                    session_id: session_id.clone(),
                    command: "echo test".to_owned(),
                    args: vec![],
                    close_on_success: false,
                },
                &ctx,
            )
            .await;

        // Then SessionTeardownFinished is still emitted.
        let events = sink.events();
        let found = events.iter().any(|e| {
            matches!(
                e,
                crate::protocol::Event::SessionTeardownFinished(
                    crate::feat::session_lifecycle::protocol::event::SessionTeardownFinished {
                        session_id: sid,
                        error: None,
                    }
                ) if sid == &session_id
            )
        });
        assert!(found, "expected SessionTeardownFinished event");
    }

    #[tokio::test]
    async fn teardown_only_success_emits_push_chat_entry() {
        // Given a session actor with a session.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let state = actor.state.read();
            state.session.active_session_id().clone()
        };

        // When teardown succeeds with close_on_success: false.
        actor
            .handle_run_session_teardown(
                &crate::feat::session_lifecycle::protocol::command::RunSessionTeardown {
                    session_id: session_id.clone(),
                    command: "echo test".to_owned(),
                    args: vec![],
                    close_on_success: false,
                },
                &ctx,
            )
            .await;

        // Then a PushChatEntry command with a System entry was emitted.
        let commands = sink.commands();
        let has_success = commands.iter().any(|cmd| {
            matches!(
                cmd,
                Command::PushChatEntry(PushChatEntry { entry, .. })
                if matches!(&entry.kind, ChatEntryKind::System(t) if t.contains("Teardown"))
            )
        });
        assert!(
            has_success,
            "expected PushChatEntry with teardown success entry"
        );
    }

    // --- SessionState / LifecycleScriptState close tests ---

    #[tokio::test]
    async fn close_session_with_nothing_ran_skips_teardown_and_archives() {
        // Given a session with LifecycleScriptState::NothingRan and history.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("hello"));
            assert_eq!(
                state.active_session().lifecycle_script_state(),
                crate::feat::session::chat_session::LifecycleScriptState::NothingRan
            );
            state.session.active_session_id().clone()
        };

        // When handling CloseSession.
        actor
            .handle_close_session(
                &crate::feat::session::protocol::close_session::CloseSession {
                    session_id: session_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then the session is removed from memory (archived).
        let state = actor.state.read();
        assert!(!state.session.sessions().contains_key(&session_id));
        // And no teardown-related events were emitted.
        let events = sink.events();
        let has_teardown_finished = events
            .iter()
            .any(|e| matches!(e, crate::protocol::Event::SessionTeardownFinished(..)));
        assert!(
            !has_teardown_finished,
            "did not expect SessionTeardownFinished for NothingRan"
        );
        // And SessionArchived was emitted.
        let has_archived = events
            .iter()
            .any(|e| matches!(e, crate::protocol::Event::SessionArchived(..)));
        assert!(has_archived, "expected SessionArchived");
    }

    #[tokio::test]
    async fn archive_session_without_lifecycle_removes_from_memory() {
        // Given a session with history and no lifecycle.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let second = ChatSessionState::new();
        let second_id = second.session_id().clone();
        let target_id = {
            let mut state = actor.state.write();
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("hello"));
            state.session.sessions_mut().insert(second_id, second);
            state.session.active_session_id().clone()
        };

        // When handling ArchiveSession for the active session.
        actor
            .handle_archive_session(
                &crate::feat::session::protocol::archive_session::ArchiveSession {
                    session_id: target_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then the session is removed from memory.
        let state = actor.state.read();
        assert!(!state.session.sessions().contains_key(&target_id));
        assert_eq!(state.session.sessions().len(), 1);
        drop(state);

        // And SessionArchived and SessionClosed are emitted.
        let events = sink.events();
        let has_archived = events.iter().any(|e| {
            matches!(
                e,
                crate::protocol::Event::SessionArchived(
                    crate::feat::session::protocol::session_archived::SessionArchived { session_id: sid }
                ) if sid == &target_id
            )
        });
        assert!(has_archived, "expected SessionArchived");

        let has_closed = events.iter().any(|e| {
            matches!(
                e,
                crate::protocol::Event::SessionClosed(
                    crate::feat::session::protocol::session_closed::SessionClosed { session_id: sid }
                ) if sid == &target_id
            )
        });
        assert!(has_closed, "expected SessionClosed");

        // And no teardown events were emitted.
        let has_teardown = events
            .iter()
            .any(|e| matches!(e, crate::protocol::Event::SessionTeardownFinished(..)));
        assert!(
            !has_teardown,
            "did not expect SessionTeardownFinished for archive"
        );
    }

    #[tokio::test]
    async fn archive_empty_session_removes_and_archives() {
        // Given an empty session.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let second = ChatSessionState::new();
        let second_id = second.session_id().clone();
        let target_id = {
            let mut state = actor.state.write();
            state.session.sessions_mut().insert(second_id, second);
            state.session.active_session_id().clone()
        };

        // When handling ArchiveSession for the empty session.
        actor
            .handle_archive_session(
                &crate::feat::session::protocol::archive_session::ArchiveSession {
                    session_id: target_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then the session is removed from memory.
        let state = actor.state.read();
        assert!(!state.session.sessions().contains_key(&target_id));
        drop(state);

        // And SessionArchived is emitted (empty sessions are still archived in DB).
        let events = sink.events();
        let has_archived = events
            .iter()
            .any(|e| matches!(e, crate::protocol::Event::SessionArchived(..)));
        assert!(
            has_archived,
            "expected SessionArchived for empty session"
        );

        let has_closed = events.iter().any(|e| {
            matches!(
                e,
                crate::protocol::Event::SessionClosed(
                    crate::feat::session::protocol::session_closed::SessionClosed { session_id: sid }
                ) if sid == &target_id
            )
        });
        assert!(has_closed, "expected SessionClosed");
    }

    #[tokio::test]
    async fn archive_active_session_switches_to_next() {
        // Given two sessions, archiving the active one.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let second = ChatSessionState::new();
        let second_id = second.session_id().clone();
        let active_id = {
            let mut state = actor.state.write();
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("msg"));
            state.session.sessions_mut().insert(second_id, second);
            state.session.active_session_id().clone()
        };

        // When archiving the active session.
        actor
            .handle_archive_session(
                &crate::feat::session::protocol::archive_session::ArchiveSession {
                    session_id: active_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then the active session switches to the remaining one.
        let state = actor.state.read();
        assert_ne!(*state.session.active_session_id(), active_id);
        assert_eq!(state.session.sessions().len(), 1);
    }

    #[tokio::test]
    async fn archive_last_session_creates_new_one() {
        // Given one non-empty session.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let only_id = {
            let mut state = actor.state.write();
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("msg"));
            state.session.active_session_id().clone()
        };

        // When archiving the only session.
        actor
            .handle_archive_session(
                &crate::feat::session::protocol::archive_session::ArchiveSession {
                    session_id: only_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then a new session is created.
        let state = actor.state.read();
        assert!(!state.session.sessions().contains_key(&only_id));
        assert_eq!(state.session.sessions().len(), 1);
        assert_ne!(*state.session.active_session_id(), only_id);
    }

    // --- Teardown failure/success during close ---

    #[tokio::test]
    async fn close_session_leaves_lifecycle_at_setup_ran_when_teardown_fails() {
        use crate::feat::preferences_actor::user_preferences::SessionLifecycle;
        use crate::feat::session::chat_session::LifecycleScriptState;

        // Given a session with SetupRan, a lifecycle, and a failing teardown command.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            session.set_lifecycle_name(Some("test".to_owned()));
            session.advance_lifecycle_after_setup();
            state.frontend.preferences.session_lifecycles = vec![SessionLifecycle {
                name: "test".to_owned(),
                description: None,
                setup_command: None,
                teardown_command: Some("exit 1".to_owned()),
            }];
            state.session.active_session_id().clone()
        };

        // When handling CloseSession.
        actor
            .handle_close_session(
                &crate::feat::session::protocol::close_session::CloseSession {
                    session_id: session_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then LifecycleScriptState is still SetupRan.
        let state = actor.state.read();
        let session = state
            .session
            .sessions()
            .get(&session_id)
            .expect("session exists");
        assert_eq!(
            session.lifecycle_script_state(),
            LifecycleScriptState::SetupRan
        );

        // And the session is still in memory.
        assert!(state.session.sessions().contains_key(&session_id));
    }

    #[tokio::test]
    async fn close_session_with_teardown_failure_pushes_error_entry() {
        use crate::feat::preferences_actor::user_preferences::SessionLifecycle;

        // Given a session with SetupRan and a failing teardown.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            session.set_lifecycle_name(Some("test".to_owned()));
            session.advance_lifecycle_after_setup();
            state.frontend.preferences.session_lifecycles = vec![SessionLifecycle {
                name: "test".to_owned(),
                description: None,
                setup_command: None,
                teardown_command: Some("exit 1".to_owned()),
            }];
            state.session.active_session_id().clone()
        };

        // When handling CloseSession.
        actor
            .handle_close_session(
                &crate::feat::session::protocol::close_session::CloseSession {
                    session_id: session_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then an error entry was emitted via PushChatEntry command.
        let commands = sink.commands();
        let has_error = commands.iter().any(|cmd| {
            matches!(
                cmd,
                Command::PushChatEntry(PushChatEntry { entry, .. })
                if matches!(entry.kind, crate::protocol::ChatEntryKind::Error(_))
            )
        });
        assert!(has_error, "expected PushChatEntry command with error entry");
    }

    #[tokio::test]
    async fn close_session_advances_lifecycle_when_teardown_succeeds() {
        use crate::feat::preferences_actor::user_preferences::SessionLifecycle;

        // Given a session with SetupRan and a succeeding teardown command.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let second_session = ChatSessionState::new();
        let session_id = {
            let mut state = actor.state.write();
            // Add a second session so close doesn't create a new one.
            state
                .session
                .sessions_mut()
                .insert(second_session.session_id().clone(), second_session);
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            session.set_lifecycle_name(Some("test".to_owned()));
            session.advance_lifecycle_after_setup();
            state.frontend.preferences.session_lifecycles = vec![SessionLifecycle {
                name: "test".to_owned(),
                description: None,
                setup_command: None,
                teardown_command: Some("exit 0".to_owned()),
            }];
            state.session.active_session_id().clone()
        };

        // When handling CloseSession.
        actor
            .handle_close_session(
                &crate::feat::session::protocol::close_session::CloseSession {
                    session_id: session_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then the session is removed from memory (close succeeded).
        let state = actor.state.read();
        assert!(!state.session.sessions().contains_key(&session_id));

        // And SessionTeardownFinished was emitted with no error.
        let events = sink.events();
        let found = events.iter().any(|e| {
            matches!(
                e,
                crate::protocol::Event::SessionTeardownFinished(
                    crate::feat::session_lifecycle::protocol::event::SessionTeardownFinished {
                        session_id: sid,
                        error: None,
                    }
                ) if sid == &session_id
            )
        });
        assert!(found, "expected SessionTeardownFinished with no error");
    }

    // --- Empty session archive/close tests ---

    #[tokio::test]
    async fn archiving_empty_session_sets_archived_flag() {
        // Given an actor with an empty session and a recording store.
        use super::super::super::helpers::{test_actor_with_store, test_context};

        let (actor, store) = test_actor_with_store(vec![]);
        let session_id = actor.state.read().session.active_session_id().clone();
        let (_sink, ctx) = test_context();

        // When handling ArchiveSession.
        actor
            .handle_archive_session(
                &crate::feat::session::protocol::archive_session::ArchiveSession {
                    session_id: session_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then the session was archived in the store.
        assert!(store.was_archived(&session_id), "empty session should be archived");
    }

    #[tokio::test]
    async fn closing_empty_session_sets_archived_flag() {
        // Given an actor with an empty session and a recording store.
        use super::super::super::helpers::{test_actor_with_store, test_context};

        let (actor, store) = test_actor_with_store(vec![]);
        let session_id = actor.state.read().session.active_session_id().clone();
        let (_sink, ctx) = test_context();

        // When handling CloseSession.
        actor
            .handle_close_session(
                &crate::feat::session::protocol::close_session::CloseSession {
                    session_id: session_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then the session was archived in the store.
        assert!(store.was_archived(&session_id), "empty session should be archived");
    }
}
