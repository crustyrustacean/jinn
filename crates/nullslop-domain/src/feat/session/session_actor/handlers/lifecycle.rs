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
use crate::feat::session_lifecycle::protocol::command::{
    PersistSession, RunSessionSetup, RunSessionTeardown,
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
pub fn setup_running_msg() -> ChatEntry {
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
        // Mark session as busy (spinner) before the .await.
        {
            let mut state = self.state.write();
            if let Some(session) = state.session.get_mut(&payload.session_id) {
                session.mark_busy();
            }
        }

        match payload.lifecycle_command {
            Some(ref cmd) => match cmd {
                crate::feat::session_lifecycle::builtin::LifecycleCommand::Builtin(id) => {
                    self.run_builtin_setup(&payload.session_id, id, &payload.args, ctx);
                }
                crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(_) => {
                    self.run_shell_setup(payload, ctx).await;
                }
            },
            None => {
                self.run_shell_setup(payload, ctx).await;
            }
        }
    }

    /// Runs a shell-based setup command via `run_setup_command`.
    async fn run_shell_setup(&self, payload: &RunSessionSetup, ctx: &ActorContext) {
        let result = run_setup_command(&payload.command).await;

        // Clear busy flag in all code paths.
        {
            let mut state = self.state.write();
            if let Some(session) = state.session.get_mut(&payload.session_id) {
                session.mark_busy_complete();
            }
        }

        match result {
            Ok(cwd) => {
                {
                    let mut state = self.state.write();
                    if let Some(session) = state.session.get_mut(&payload.session_id) {
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
                    if let Some(session) = state.session.get_mut(&payload.session_id) {
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

    /// Runs a builtin lifecycle setup by looking up the handler in the registry.
    fn run_builtin_setup(
        &self,
        session_id: &crate::protocol::SessionId,
        id: &crate::feat::session_lifecycle::builtin::BuiltinId,
        args: &[String],
        ctx: &ActorContext,
    ) {
        let Some(handler) = self.builtin_registry.get(id) else {
            let error_msg = format!("unknown builtin lifecycle: {id}");
            tracing::error!(%id, "builtin handler not found in registry");

            let default_cwd = {
                let mut state = self.state.write();
                let default = state.session.default_cwd().clone();
                if let Some(session) = state.session.get_mut(session_id) {
                    session.set_cwd(default.clone());
                }
                default
            };

            if let Err(e) = ctx.send_command(Command::PushChatEntry(PushChatEntry {
                session_id: session_id.clone(),
                entry: ChatEntry::error(&error_msg),
            })) {
                tracing::warn!(err = ?e, "session-actor failed to emit PushChatEntry for builtin setup error");
            }

            if let Err(e) = ctx.send_event(Event::SessionSetupCompleted(SessionSetupCompleted {
                session_id: session_id.clone(),
                cwd: default_cwd,
                error: Some(error_msg),
            })) {
                tracing::warn!(err = ?e, "session-actor failed to emit SessionSetupCompleted");
            }
            return;
        };

        match handler.setup(session_id, args) {
            Ok(cwd) => {
                {
                    let mut state = self.state.write();
                    if let Some(session) = state.session.get_mut(session_id) {
                        session.set_cwd(cwd.clone());
                        session.advance_lifecycle_after_setup();
                    }
                }

                if let Err(e) = ctx.send_command(Command::PushChatEntry(PushChatEntry {
                    session_id: session_id.clone(),
                    entry: setup_complete_msg(&cwd),
                })) {
                    tracing::warn!(err = ?e, "session-actor failed to emit PushChatEntry for builtin setup complete");
                }

                if let Err(e) =
                    ctx.send_event(Event::SessionSetupCompleted(SessionSetupCompleted {
                        session_id: session_id.clone(),
                        cwd,
                        error: None,
                    }))
                {
                    tracing::warn!(err = ?e, "session-actor failed to emit SessionSetupCompleted");
                }
            }
            Err(report) => {
                let error_msg = format!("builtin setup failed: {report:#?}");
                let default_cwd = {
                    let mut state = self.state.write();
                    let default = state.session.default_cwd().clone();
                    if let Some(session) = state.session.get_mut(session_id) {
                        session.set_cwd(default.clone());
                        session.mark_busy_complete();
                    }
                    default
                };

                if let Err(e) = ctx.send_command(Command::PushChatEntry(PushChatEntry {
                    session_id: session_id.clone(),
                    entry: ChatEntry::error(&error_msg),
                })) {
                    tracing::warn!(err = ?e, "session-actor failed to emit PushChatEntry for builtin setup error");
                }

                if let Err(e) =
                    ctx.send_event(Event::SessionSetupCompleted(SessionSetupCompleted {
                        session_id: session_id.clone(),
                        cwd: default_cwd,
                        error: Some(error_msg),
                    }))
                {
                    tracing::warn!(err = ?e, "session-actor failed to emit SessionSetupCompleted");
                }
            }
        }
    }

    /// RunSessionTeardown: teardown-only handler (`t` key).
    ///
    /// For shell teardowns: sets `TearingDown` phase, spawns a tokio task
    /// to run the shell command, and returns immediately. The spawned task
    /// sends `FinishSessionTeardown` back when complete.
    ///
    /// For builtin teardowns: runs inline (synchronous, no blocking).
    #[allow(clippy::too_many_lines, clippy::items_after_statements)]
    pub(in crate::feat::session::session_actor) async fn handle_run_session_teardown(
        &self,
        payload: &RunSessionTeardown,
        ctx: &ActorContext,
    ) {
        // Look up teardown command from session's lifecycle.
        let (teardown_cmd, lifecycle_args) = {
            let state = self.state.read();
            let Some(session) = state.session.get(&payload.session_id) else {
                return;
            };
            let lifecycle_name = session.lifecycle_name().map(str::to_owned);
            let args = session.lifecycle_args().to_vec();
            let teardown = lifecycle_name.as_deref().and_then(|name| {
                state
                    .frontend
                    .preferences
                    .session_lifecycles
                    .iter()
                    .find(|l| l.name == name)
                    .and_then(|l| l.teardown.clone())
            });
            (teardown, args)
        };

        let Some(ref teardown_cmd) = teardown_cmd else {
            return;
        };

        match teardown_cmd {
            crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(shell_cmd) => {
                // Set TearingDown phase + mark busy.
                let (old_phase, rendered) = {
                    let mut state = self.state.write();
                    let Some(session) = state.session.get_mut(&payload.session_id) else {
                        return;
                    };
                    let old_phase = session.phase();
                    session.begin_tearing_down();
                    session.mark_busy();
                    use crate::feat::session_lifecycle::command_template::CommandTemplate;
                    let template = CommandTemplate::parse(shell_cmd);
                    let rendered = if lifecycle_args.is_empty() {
                        shell_cmd.clone()
                    } else {
                        template.render(&lifecycle_args)
                    };
                    (old_phase, rendered)
                };

                // Push "running" entry + emit phase change.
                self.push_and_save(&payload.session_id, teardown_running_msg(), ctx)
                    .await;
                super::super::helpers::emit_phase_changed(
                    ctx,
                    &payload.session_id,
                    old_phase,
                    crate::feat::session::chat_session::SessionPhase::TearingDown,
                );

                // Spawn tokio task to run the shell command.
                let session_id = payload.session_id.clone();
                let sink = ctx.sink();
                tokio::spawn(async move {
                    let result =
                        crate::feat::session_lifecycle::command_runner::run_teardown_command(
                            &rendered,
                        )
                        .await;
                    let error = result.err().map(|report| {
                        if let Some(cmd_err) =
                            report.downcast_ref::<crate::feat::session_lifecycle::command_runner::LifecycleCommandError>()
                        {
                            crate::feat::session::session_actor::handlers::lifecycle::format_lifecycle_error(cmd_err)
                        } else {
                            crate::feat::session::session_actor::handlers::lifecycle::strip_ansi(&format!(
                                "{report:#?}"
                            ))
                        }
                    });
                    let _ = sink.send_command(crate::protocol::Command::FinishSessionTeardown(
                        crate::feat::session_lifecycle::protocol::command::FinishSessionTeardown {
                            session_id,
                            close_after: false,
                            error,
                        },
                    ));
                });
            }
            crate::feat::session_lifecycle::builtin::LifecycleCommand::Builtin(id) => {
                // Builtin teardown is synchronous — run inline.
                let success = self
                    .run_builtin_teardown(&payload.session_id, id, &lifecycle_args, ctx)
                    .await;

                if !success {
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

                // Push success entry via PushChatEntry (persists automatically).
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
    }

    /// Runs a builtin lifecycle teardown by looking up the handler in the registry.
    ///
    /// Returns `true` if teardown succeeded, `false` if it failed.
    /// Advances `lifecycle_script_state` to `TeardownRan` on success.
    async fn run_builtin_teardown(
        &self,
        session_id: &crate::protocol::SessionId,
        id: &crate::feat::session_lifecycle::builtin::BuiltinId,
        args: &[String],
        ctx: &ActorContext,
    ) -> bool {
        let Some(handler) = self.builtin_registry.get(id) else {
            let error_msg = format!("unknown builtin lifecycle: {id}");
            tracing::error!(%id, "builtin handler not found in registry for teardown");

            if let Err(e) = ctx.send_command(Command::PushChatEntry(PushChatEntry {
                session_id: session_id.clone(),
                entry: ChatEntry::error(&error_msg),
            })) {
                tracing::warn!(err = ?e, "session-actor failed to emit PushChatEntry for builtin teardown error");
            }
            return false;
        };

        if handler.teardown(session_id, args) {
            // Advance lifecycle_script_state: SetupRan → TeardownRan.
            {
                let mut state = self.state.write();
                if let Some(session) = state.session.get_mut(session_id) {
                    session.advance_lifecycle_after_teardown();
                }
            }
            self.save_active_session(session_id).await;
            true
        } else {
            let error_msg = format!("builtin teardown failed for: {id}");
            if let Err(e) = ctx.send_command(Command::PushChatEntry(PushChatEntry {
                session_id: session_id.clone(),
                entry: ChatEntry::error(&error_msg),
            })) {
                tracing::warn!(err = ?e, "session-actor failed to emit PushChatEntry for builtin teardown failure");
            }
            false
        }
    }

    /// CloseSession: close handler (`x` key).
    ///
    /// Linear flow:
    /// 1. If `SetupRan`: run teardown via `run_teardown_step` (which advances + persists)
    /// 2. Set `session_state = Archived`, persist via `save_active_session`
    /// 3. `remove_and_replace` from HashMap
    /// 4. Emit `SessionArchived` + `SessionClosed`
    ///
    /// On teardown failure, emits `SessionTeardownFinished(error)` and returns early.
    /// The session remains in memory with `SetupRan`.
    #[allow(clippy::too_many_lines)]
    pub(in crate::feat::session::session_actor) async fn handle_close_session(
        &self,
        payload: &CloseSession,
        ctx: &ActorContext,
    ) {
        use crate::feat::session::chat_session::{LifecycleScriptState, SessionState};

        // Collect session info under read lock.
        let session_info = {
            let state = self.state.read();
            let Some(session) = state.session.get(&payload.session_id) else {
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
                    .and_then(|l| l.teardown.clone())
            });

            if let Some(teardown_cmd) = teardown_cmd {
                match teardown_cmd {
                    crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(shell_cmd) => {
                        // For shell teardowns: set TearingDown phase, spawn tokio task,
                        // then return immediately. The spawned task signals completion
                        // via FinishSessionTeardown with close_after: true.
                        let (old_phase, rendered) = {
                            use crate::feat::session_lifecycle::command_template::CommandTemplate;

                            let mut state = self.state.write();
                            let Some(session) = state.session.get_mut(&payload.session_id) else {
                                return;
                            };
                            let old_phase = session.phase();
                            session.begin_tearing_down();
                            session.mark_busy();
                            let template = CommandTemplate::parse(&shell_cmd);
                            let rendered = if lifecycle_args.is_empty() {
                                shell_cmd.clone()
                            } else {
                                template.render(&lifecycle_args)
                            };
                            (old_phase, rendered)
                        };

                        self.push_and_save(&payload.session_id, teardown_running_msg(), ctx)
                            .await;
                        super::super::helpers::emit_phase_changed(
                            ctx,
                            &payload.session_id,
                            old_phase,
                            crate::feat::session::chat_session::SessionPhase::TearingDown,
                        );

                        let session_id = payload.session_id.clone();
                        let sink = ctx.sink();
                        tokio::spawn(async move {
                            let result = crate::feat::session_lifecycle::command_runner::run_teardown_command(
                                &rendered,
                            )
                            .await;
                            let error = result.err().map(|report| {
                                if let Some(cmd_err) =
                                    report.downcast_ref::<crate::feat::session_lifecycle::command_runner::LifecycleCommandError>()
                                {
                                    crate::feat::session::session_actor::handlers::lifecycle::format_lifecycle_error(cmd_err)
                                } else {
                                    crate::feat::session::session_actor::handlers::lifecycle::strip_ansi(&format!(
                                        "{report:#?}"
                                    ))
                                }
                            });
                            let _ = sink.send_command(
                                crate::protocol::Command::FinishSessionTeardown(
                                    crate::feat::session_lifecycle::protocol::command::FinishSessionTeardown {
                                        session_id,
                                        close_after: true,
                                        error,
                                    },
                                ),
                            );
                        });

                        return; // Return immediately — async result handled via FinishSessionTeardown
                    }
                    crate::feat::session_lifecycle::builtin::LifecycleCommand::Builtin(id) => {
                        let success = self
                            .run_builtin_teardown(&payload.session_id, &id, &lifecycle_args, ctx)
                            .await;

                        if !success {
                            return;
                        }
                    }
                }
            }
            // If no teardown command exists but state is SetupRan, skip teardown and proceed.
        }

        // Step 2: Archive + persist.
        {
            let mut state = self.state.write();
            if let Some(session) = state.session.get_mut(&payload.session_id) {
                session.set_session_state(SessionState::Archived);
            }
        }
        self.save_active_session(&payload.session_id).await;

        // Step 3: Remove from memory.
        self.remove_and_replace(&payload.session_id);

        // Step 4: Notify.
        if let Err(e) = ctx.send_event(Event::SessionArchived(SessionArchived {
            session_id: payload.session_id.clone(),
        })) {
            tracing::warn!(err = ?e, "session-actor failed to emit SessionArchived");
        }

        if let Err(e) = ctx.send_event(Event::SessionClosed(SessionClosed {
            session_id: payload.session_id.clone(),
        })) {
            tracing::warn!(err = ?e, "session-actor failed to emit SessionClosed");
        }
    }

    /// Handle `FinishSessionTeardown` — completion of an async teardown shell command.
    ///
    /// Called by the spawned tokio task after the teardown shell command finishes.
    /// Depending on `payload.close_after`:
    /// - `false` (teardown-only, `t` key): advance lifecycle state, persist, emit events
    /// - `true` (close-with-teardown, `x` key): archive and remove the session
    ///
    /// On error, an error entry is pushed and the session returns to `Idle` phase.
    pub(in crate::feat::session::session_actor) async fn handle_finish_session_teardown(
        &self,
        payload: &crate::feat::session_lifecycle::protocol::command::FinishSessionTeardown,
        ctx: &ActorContext,
    ) {
        use crate::feat::session::chat_session::SessionState;

        // Clear busy flag.
        {
            let mut state = self.state.write();
            let Some(session) = state.session.get_mut(&payload.session_id) else {
                return;
            };
            session.mark_busy_complete();
        }

        if let Some(ref error_msg) = payload.error {
            // Teardown failed — push error entry, emit failure, reset phase.
            let _ = ctx.send_command(Command::PushChatEntry(PushChatEntry {
                session_id: payload.session_id.clone(),
                entry: ChatEntry::error(format!("Teardown failed: {error_msg}")),
            }));

            let _ = ctx.send_event(Event::SessionTeardownFinished(SessionTeardownFinished {
                session_id: payload.session_id.clone(),
                error: Some(error_msg.clone()),
            }));

            // Reset phase to Idle so any queued messages drain via QueueActor.
            let old_phase = {
                let mut state = self.state.write();
                let Some(session) = state.session.get_mut(&payload.session_id) else {
                    return;
                };
                let old_phase = session.phase();
                session.finish_tearing_down();
                old_phase
            };
            super::super::helpers::emit_phase_changed(
                ctx,
                &payload.session_id,
                old_phase,
                crate::feat::session::chat_session::SessionPhase::Idle,
            );
            return;
        }

        // Teardown succeeded.
        if payload.close_after {
            // Close-with-teardown: advance lifecycle, then archive and remove.
            {
                let mut state = self.state.write();
                let Some(session) = state.session.get_mut(&payload.session_id) else {
                    return;
                };
                session.advance_lifecycle_after_teardown();
                session.finish_tearing_down();
            }

            // Persist the lifecycle state change.
            self.save_active_session(&payload.session_id).await;

            // Archive + remove.
            {
                let mut state = self.state.write();
                if let Some(session) = state.session.get_mut(&payload.session_id) {
                    session.set_session_state(SessionState::Archived);
                }
            }
            self.save_active_session(&payload.session_id).await;
            self.remove_and_replace(&payload.session_id);

            // Emit events.
            let _ = ctx.send_event(Event::SessionArchived(SessionArchived {
                session_id: payload.session_id.clone(),
            }));
            let _ = ctx.send_event(Event::SessionClosed(SessionClosed {
                session_id: payload.session_id.clone(),
            }));
            let _ = ctx.send_event(Event::SessionTeardownFinished(SessionTeardownFinished {
                session_id: payload.session_id.clone(),
                error: None,
            }));
        } else {
            // Teardown-only: advance lifecycle, persist, push success entry, emit.
            {
                let mut state = self.state.write();
                let Some(session) = state.session.get_mut(&payload.session_id) else {
                    return;
                };
                session.advance_lifecycle_after_teardown();
                session.finish_tearing_down();
            }

            // Persist the lifecycle state change.
            self.save_active_session(&payload.session_id).await;

            // Push success entry.
            let _ = ctx.send_command(Command::PushChatEntry(PushChatEntry {
                session_id: payload.session_id.clone(),
                entry: teardown_success_msg(),
            }));

            // Emit completion event.
            let _ = ctx.send_event(Event::SessionTeardownFinished(SessionTeardownFinished {
                session_id: payload.session_id.clone(),
                error: None,
            }));

            // Emit phase change so QueueActor drains queued messages.
            let old_phase = crate::feat::session::chat_session::SessionPhase::TearingDown;
            super::super::helpers::emit_phase_changed(
                ctx,
                &payload.session_id,
                old_phase,
                crate::feat::session::chat_session::SessionPhase::Idle,
            );
        }
    }

    /// ArchiveSession: archive without running teardown.
    ///
    /// Linear flow:
    /// 1. Set `session_state = Archived`, persist via `save_active_session`
    /// 2. `remove_and_replace` from HashMap
    /// 3. Emit `SessionArchived` + `SessionClosed`
    ///
    /// Does NOT check or advance `LifecycleScriptState`. The session is simply
    /// put away — it can be unarchived later.
    pub(in crate::feat::session::session_actor) async fn handle_archive_session(
        &self,
        payload: &ArchiveSession,
        ctx: &ActorContext,
    ) {
        use crate::feat::session::chat_session::SessionState;

        // Step 1: Archive + persist.
        {
            let mut state = self.state.write();
            if let Some(session) = state.session.get_mut(&payload.session_id) {
                session.set_session_state(SessionState::Archived);
            }
        }
        self.save_active_session(&payload.session_id).await;

        // Step 2: Remove from memory.
        self.remove_and_replace(&payload.session_id);

        // Step 3: Notify.
        if let Err(e) = ctx.send_event(Event::SessionArchived(SessionArchived {
            session_id: payload.session_id.clone(),
        })) {
            tracing::warn!(err = ?e, "session-actor failed to emit SessionArchived");
        }

        if let Err(e) = ctx.send_event(Event::SessionClosed(SessionClosed {
            session_id: payload.session_id.clone(),
        })) {
            tracing::warn!(err = ?e, "session-actor failed to emit SessionClosed");
        }
    }

    /// Runs the teardown command and advances lifecycle state.
    ///
    /// This is the shared teardown helper used by both the close (`x`) and
    /// teardown-only (`t`) flows. On success, it:
    /// 1. Runs the teardown command
    /// 2. Advances `lifecycle_script_state` to `TeardownRan`
    /// 3. Persists the session via `save_active_session`
    ///
    /// Returns `true` on success, `false` on failure (error entry pushed to session).
    /// Remove session from HashMap, create replacement if empty, reconcile cursor.
    ///
    /// Pure state mutation helper. Does NOT emit events — callers handle notifications.
    /// Delegates cursor and active-session reconciliation to
    /// [`reconcile_after_session_removal`].
    ///
    /// [`reconcile_after_session_removal`]: crate::feat::ui::sidebar::sessions::reconcile_after_session_removal
    pub(in crate::feat::session::session_actor) fn remove_and_replace(
        &self,
        session_id: &crate::protocol::SessionId,
    ) {
        let mut state = self.state.write();

        // Update visual-parent index before removing the session
        // (need it in memory to resolve its parent chain).
        crate::feat::ui::sidebar::sessions::update_visual_parents_on_removal(
            &mut state,
            session_id,
        );

        let fresh_session = {
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
            ChatSessionState::new_with_profile(
                crate::feat::session::profile::SessionProfile::from_config(
                    model,
                    strategy,
                    token_budget,
                    sliding_window_size,
                ),
            )
        };
        state.session.remove_and_replace(session_id, fresh_session);

        crate::feat::ui::sidebar::sessions::reconcile_after_session_removal(&mut state);
    }

    /// PersistSession: persist the session immediately.
    ///
    /// Saves the full session blob (history, lifecycle state, archive flag)
    /// to SQLite. Used by multiple flows: session creation, teardown, archive.
    pub(in crate::feat::session::session_actor) async fn handle_persist_session(
        &self,
        payload: &PersistSession,
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
        use crate::feat::preferences_actor::user_preferences::SessionLifecycle;

        let actor = test_actor();
        let (sink, ctx) = test_context();
        let (target_id, original_active) = {
            let mut state = actor.state.write();
            let original_active = state.session.active_session_id().clone();
            let second = ChatSessionState::new();
            let second_id = second.session_id().clone();
            state.session.insert(second);
            state.frontend.preferences.session_lifecycles = vec![SessionLifecycle {
                name: "test".to_owned(),
                description: None,
                setup: None,
                teardown: Some(
                    crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(
                        "exit 1".to_owned(),
                    ),
                ),
            }];
            let session = state.session.get_mut(&second_id).expect("second session");
            session.set_lifecycle_name(Some("test".to_owned()));
            (second_id, original_active)
        };

        // When handling a teardown command that fails targeting the non-active session.
        actor
            .handle_run_session_teardown(
                &crate::feat::session_lifecycle::protocol::command::RunSessionTeardown {
                    session_id: target_id.clone(),
                    command: "exit 1".to_owned(),
                    args: vec![],
                },
                &ctx,
            )
            .await;

        // Simulate async teardown failure.
        let finish = crate::feat::session_lifecycle::protocol::command::FinishSessionTeardown {
            session_id: target_id.clone(),
            close_after: false,
            error: Some("teardown failed".to_owned()),
        };
        actor.handle_finish_session_teardown(&finish, &ctx).await;

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
        // Given a session actor in Normal scope with a lifecycle.
        use crate::feat::preferences_actor::user_preferences::SessionLifecycle;

        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            state
                .active_session_mut()
                .set_lifecycle_name(Some("test".to_owned()));
            state.frontend.preferences.session_lifecycles = vec![SessionLifecycle {
                name: "test".to_owned(),
                description: None,
                setup: None,
                teardown: Some(
                    crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(
                        "exit 1".to_owned(),
                    ),
                ),
            }];
            state.session.active_session_id().clone()
        };

        // When handling a teardown command that fails.
        actor
            .handle_run_session_teardown(
                &crate::feat::session_lifecycle::protocol::command::RunSessionTeardown {
                    session_id,
                    command: "exit 1".to_owned(),
                    args: vec![],
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
        // Given a session actor with a session and a lifecycle.
        use crate::feat::preferences_actor::user_preferences::SessionLifecycle;

        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            state
                .active_session_mut()
                .set_lifecycle_name(Some("test".to_owned()));
            state.frontend.preferences.session_lifecycles = vec![SessionLifecycle {
                name: "test".to_owned(),
                description: None,
                setup: None,
                teardown: Some(
                    crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(
                        "exit 1".to_owned(),
                    ),
                ),
            }];
            state.session.active_session_id().clone()
        };

        // When handling a teardown command that fails.
        actor
            .handle_run_session_teardown(
                &crate::feat::session_lifecycle::protocol::command::RunSessionTeardown {
                    session_id: session_id.clone(),
                    command: "exit 1".to_owned(),
                    args: vec![],
                },
                &ctx,
            )
            .await;

        // Simulate async teardown failure.
        let finish = crate::feat::session_lifecycle::protocol::command::FinishSessionTeardown {
            session_id: session_id.clone(),
            close_after: false,
            error: Some("exit code 1".to_owned()),
        };
        actor.handle_finish_session_teardown(&finish, &ctx).await;

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
        // Given a session actor with a session and a lifecycle.
        use crate::feat::preferences_actor::user_preferences::SessionLifecycle;

        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            state
                .active_session_mut()
                .set_lifecycle_name(Some("test".to_owned()));
            state.frontend.preferences.session_lifecycles = vec![SessionLifecycle {
                name: "test".to_owned(),
                description: None,
                setup: None,
                teardown: Some(
                    crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(
                        "exit 1".to_owned(),
                    ),
                ),
            }];
            state.session.active_session_id().clone()
        };

        // When handling a teardown command that fails.
        actor
            .handle_run_session_teardown(
                &crate::feat::session_lifecycle::protocol::command::RunSessionTeardown {
                    session_id: session_id.clone(),
                    command: "exit 1".to_owned(),
                    args: vec![],
                },
                &ctx,
            )
            .await;

        // Simulate async teardown failure.
        let finish = crate::feat::session_lifecycle::protocol::command::FinishSessionTeardown {
            session_id: session_id.clone(),
            close_after: false,
            error: Some("teardown failed".to_owned()),
        };
        actor.handle_finish_session_teardown(&finish, &ctx).await;

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
            state.session.insert(second);
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
        assert!(!state.session.contains(&second_id));
        // And the first session still exists.
        assert_eq!(state.session.session_count(), 1);
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
        assert!(!state.session.contains(&only_id));
        assert_eq!(state.session.session_count(), 1);
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
            state.session.insert(second);
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
        assert_eq!(state.session.session_count(), 1);
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
            state.session.insert(second);
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
            state.session.session_count()
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
        assert_eq!(state.session.session_count(), original_len);
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

    // --- Teardown-only (t key) tests ---

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
            state.session.session_count()
        };

        // When teardown succeeds.
        actor
            .handle_run_session_teardown(
                &crate::feat::session_lifecycle::protocol::command::RunSessionTeardown {
                    session_id: session_id.clone(),
                    command: "echo test".to_owned(),
                    args: vec![],
                },
                &ctx,
            )
            .await;

        // Then the session is NOT removed.
        let state = actor.state.read();
        assert!(state.session.contains(&session_id));
        assert_eq!(state.session.session_count(), original_count);
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

        // When teardown succeeds.
        actor
            .handle_run_session_teardown(
                &crate::feat::session_lifecycle::protocol::command::RunSessionTeardown {
                    session_id: session_id.clone(),
                    command: "echo test".to_owned(),
                    args: vec![],
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
        // Given a session actor with a session and a lifecycle.
        use crate::feat::preferences_actor::user_preferences::SessionLifecycle;

        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            state
                .active_session_mut()
                .set_lifecycle_name(Some("test".to_owned()));
            state.active_session_mut().advance_lifecycle_after_setup();
            state.frontend.preferences.session_lifecycles = vec![SessionLifecycle {
                name: "test".to_owned(),
                description: None,
                setup: None,
                teardown: Some(
                    crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(
                        "echo test".to_owned(),
                    ),
                ),
            }];
            state.session.active_session_id().clone()
        };

        // When teardown succeeds.
        actor
            .handle_run_session_teardown(
                &crate::feat::session_lifecycle::protocol::command::RunSessionTeardown {
                    session_id: session_id.clone(),
                    command: "echo test".to_owned(),
                    args: vec![],
                },
                &ctx,
            )
            .await;

        // Simulate async teardown completion.
        let finish = crate::feat::session_lifecycle::protocol::command::FinishSessionTeardown {
            session_id: session_id.clone(),
            close_after: false,
            error: None,
        };
        actor.handle_finish_session_teardown(&finish, &ctx).await;

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
        // Given a session actor with a session and a lifecycle.
        use crate::feat::preferences_actor::user_preferences::SessionLifecycle;

        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            state
                .active_session_mut()
                .set_lifecycle_name(Some("test".to_owned()));
            state.active_session_mut().advance_lifecycle_after_setup();
            state.frontend.preferences.session_lifecycles = vec![SessionLifecycle {
                name: "test".to_owned(),
                description: None,
                setup: None,
                teardown: Some(
                    crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(
                        "echo test".to_owned(),
                    ),
                ),
            }];
            state.session.active_session_id().clone()
        };

        // When teardown succeeds.
        actor
            .handle_run_session_teardown(
                &crate::feat::session_lifecycle::protocol::command::RunSessionTeardown {
                    session_id: session_id.clone(),
                    command: "echo test".to_owned(),
                    args: vec![],
                },
                &ctx,
            )
            .await;

        // Simulate async teardown completion.
        let finish = crate::feat::session_lifecycle::protocol::command::FinishSessionTeardown {
            session_id: session_id.clone(),
            close_after: false,
            error: None,
        };
        actor.handle_finish_session_teardown(&finish, &ctx).await;

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
        assert!(!state.session.contains(&session_id));
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
        let _second_id = second.session_id().clone();
        let target_id = {
            let mut state = actor.state.write();
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("hello"));
            state.session.insert(second);
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
        assert!(!state.session.contains(&target_id));
        assert_eq!(state.session.session_count(), 1);
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
        let _second_id = second.session_id().clone();
        let target_id = {
            let mut state = actor.state.write();
            state.session.insert(second);
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
        assert!(!state.session.contains(&target_id));
        drop(state);

        // And SessionArchived is emitted (empty sessions are still archived in DB).
        let events = sink.events();
        let has_archived = events
            .iter()
            .any(|e| matches!(e, crate::protocol::Event::SessionArchived(..)));
        assert!(has_archived, "expected SessionArchived for empty session");

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
        let _second_id = second.session_id().clone();
        let active_id = {
            let mut state = actor.state.write();
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("msg"));
            state.session.insert(second);
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
        assert_eq!(state.session.session_count(), 1);
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
        assert!(!state.session.contains(&only_id));
        assert_eq!(state.session.session_count(), 1);
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
                setup: None,
                teardown: Some(
                    crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(
                        "exit 1".to_owned(),
                    ),
                ),
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

        // Simulate async teardown failure (close_after: true, error).
        let finish = crate::feat::session_lifecycle::protocol::command::FinishSessionTeardown {
            session_id: session_id.clone(),
            close_after: true,
            error: Some("exit code 1".to_owned()),
        };
        actor.handle_finish_session_teardown(&finish, &ctx).await;

        // Then LifecycleScriptState is still SetupRan.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert_eq!(
            session.lifecycle_script_state(),
            LifecycleScriptState::SetupRan
        );

        // And the session is still in memory.
        assert!(state.session.contains(&session_id));
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
                setup: None,
                teardown: Some(
                    crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(
                        "exit 1".to_owned(),
                    ),
                ),
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

        // Simulate async teardown failure (close_after: true, error).
        let finish = crate::feat::session_lifecycle::protocol::command::FinishSessionTeardown {
            session_id: session_id.clone(),
            close_after: true,
            error: Some("teardown failed".to_owned()),
        };
        actor.handle_finish_session_teardown(&finish, &ctx).await;

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
            state.session.insert(second_session);
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            session.set_lifecycle_name(Some("test".to_owned()));
            session.advance_lifecycle_after_setup();
            state.frontend.preferences.session_lifecycles = vec![SessionLifecycle {
                name: "test".to_owned(),
                description: None,
                setup: None,
                teardown: Some(
                    crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(
                        "exit 0".to_owned(),
                    ),
                ),
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

        // Simulate async teardown completion (close_after: true, success).
        let finish = crate::feat::session_lifecycle::protocol::command::FinishSessionTeardown {
            session_id: session_id.clone(),
            close_after: true,
            error: None,
        };
        actor.handle_finish_session_teardown(&finish, &ctx).await;

        // Then the session is removed from memory (close succeeded).
        let state = actor.state.read();
        assert!(!state.session.contains(&session_id));

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
        use crate::feat::session::chat_session::SessionState;

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

        // Then the session was saved with Archived state.
        let saved = store
            .last_saved_session(&session_id)
            .expect("session should have been saved");
        assert_eq!(saved.session_state(), SessionState::Archived);
    }

    #[tokio::test]
    async fn closing_empty_session_sets_archived_flag() {
        // Given an actor with an empty session and a recording store.
        use super::super::super::helpers::{test_actor_with_store, test_context};
        use crate::feat::session::chat_session::SessionState;

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

        // Then the session was saved with Archived state.
        let saved = store
            .last_saved_session(&session_id)
            .expect("session should have been saved");
        assert_eq!(saved.session_state(), SessionState::Archived);
    }

    // --- Teardown persistence tests ---

    #[tokio::test]
    async fn teardown_only_advances_lifecycle_to_teardown_ran() {
        use crate::feat::preferences_actor::user_preferences::SessionLifecycle;
        use crate::feat::session::chat_session::LifecycleScriptState;

        // Given a session with SetupRan and a succeeding teardown command.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.set_lifecycle_name(Some("test".to_owned()));
            session.advance_lifecycle_after_setup();
            state.frontend.preferences.session_lifecycles = vec![SessionLifecycle {
                name: "test".to_owned(),
                description: None,
                setup: None,
                teardown: Some(
                    crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(
                        "exit 0".to_owned(),
                    ),
                ),
            }];
            state.session.active_session_id().clone()
        };

        // When handling RunSessionTeardown.
        actor
            .handle_run_session_teardown(
                &crate::feat::session_lifecycle::protocol::command::RunSessionTeardown {
                    session_id: session_id.clone(),
                    command: "exit 0".to_owned(),
                    args: vec![],
                },
                &ctx,
            )
            .await;

        // Simulate async teardown completion.
        let finish = crate::feat::session_lifecycle::protocol::command::FinishSessionTeardown {
            session_id: session_id.clone(),
            close_after: false,
            error: None,
        };
        actor.handle_finish_session_teardown(&finish, &ctx).await;

        // Then lifecycle_script_state is TeardownRan.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert_eq!(
            session.lifecycle_script_state(),
            LifecycleScriptState::TeardownRan
        );

        // And the session is still in memory (not removed).
        assert!(state.session.contains(&session_id));
    }

    #[tokio::test]
    async fn close_session_with_setup_ran_persists_teardown_ran() {
        use super::super::super::helpers::{test_actor_with_store, test_context};
        use crate::feat::preferences_actor::user_preferences::SessionLifecycle;
        use crate::feat::session::chat_session::LifecycleScriptState;

        // Given a session with SetupRan, a succeeding teardown, and a recording store.
        let (actor, store) = test_actor_with_store(vec![]);
        let second = ChatSessionState::new();
        let session_id = {
            let mut state = actor.state.write();
            state.session.insert(second);
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            session.set_lifecycle_name(Some("test".to_owned()));
            session.advance_lifecycle_after_setup();
            state.frontend.preferences.session_lifecycles = vec![SessionLifecycle {
                name: "test".to_owned(),
                description: None,
                setup: None,
                teardown: Some(
                    crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(
                        "exit 0".to_owned(),
                    ),
                ),
            }];
            state.session.active_session_id().clone()
        };
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

        // Simulate async teardown completion (close_after: true).
        let finish = crate::feat::session_lifecycle::protocol::command::FinishSessionTeardown {
            session_id: session_id.clone(),
            close_after: true,
            error: None,
        };
        actor.handle_finish_session_teardown(&finish, &ctx).await;

        // Then the session was saved with TeardownRan lifecycle state.
        let saved = store
            .last_saved_session(&session_id)
            .expect("session should have been saved");
        assert_eq!(
            saved.lifecycle_script_state(),
            LifecycleScriptState::TeardownRan
        );
    }
}
