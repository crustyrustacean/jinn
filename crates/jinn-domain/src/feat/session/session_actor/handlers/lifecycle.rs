//! Session lifecycle handlers - manage setup, teardown, close, and archive operations.
//!
//! Handles the full session lifecycle: running setup/teardown commands, closing sessions
//! (with optional teardown), archiving sessions to SQLite, and creating replacement sessions
//! when the last one is removed. Also contains the helper functions for building lifecycle
//! chat entries and formatting command errors.

use crate::common::actor_deps::BusPublish;
use crate::feat::chat_input::protocol::command::PushChatEntry;
use crate::feat::chat_input::protocol::event::ChatEntrySubmitted;
use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::session::protocol::archive_session::ArchiveSession;
use crate::feat::session::protocol::close_session::CloseSession;
use crate::feat::session::protocol::session_archived::SessionArchived;
use crate::feat::session::protocol::session_closed::SessionClosed;
use crate::feat::session_lifecycle::command_runner::LifecycleCommandError;

use crate::feat::session_lifecycle::protocol::FinishSessionSetup;
use crate::feat::session_lifecycle::protocol::command::{
    FinishSessionTeardown, PersistSession, RunSessionSetup, RunSessionTeardown, SetSessionCwd,
};
use crate::feat::session_lifecycle::protocol::event::{
    SessionCwdChanged, SessionSetupCompleted, SessionTeardownFinished,
};
use crate::protocol::ChatEntry;

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
        "✅ Setup complete - Using {} as cwd",
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
    ) {
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(session_id);
            session.push_entry(entry.clone());
        }
        self.publish(ChatEntrySubmitted {
            session_id: session_id.clone(),
            entry,
        })
        .await;
        self.save_active_session(session_id).await;
    }

    /// RunSessionSetup: execute the lifecycle setup command asynchronously.
    ///
    /// On success, sets the session's CWD to the command's output.
    /// On failure, sets the default CWD and pushes an error entry.
    pub(in crate::feat::session::session_actor) async fn handle_run_session_setup(
        &mut self,
        payload: &RunSessionSetup,
    ) {
        // Mark session as busy.
        {
            let mut state = self.state.write();
            if let Some(session) = state.session.get_mut(&payload.session_id) {
                session.begin_busy();
            }
        }

        match payload.lifecycle_command {
            Some(ref cmd) => match cmd {
                crate::feat::session_lifecycle::builtin::LifecycleCommand::Builtin(id) => {
                    // Builtin: run inline, then complete Working.
                    self.run_builtin_setup(&payload.session_id, id, &payload.args)
                        .await;
                    // Complete busy.
                    {
                        let mut state = self.state.write();
                        if let Some(session) = state.session.get_mut(&payload.session_id) {
                            session.complete_busy();
                        }
                    }
                }
                crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(_) => {
                    self.spawn_shell_setup(payload).await;
                }
            },
            None => {
                self.spawn_shell_setup(payload).await;
            }
        }
    }

    async fn spawn_shell_setup(&mut self, payload: &RunSessionSetup) {
        use crate::feat::session_lifecycle::command_runner::spawn_setup_command;

        // Read the session's CWD before spawning so the script runs in the
        // inherited session dir, not jinn's process dir. Falls back to the
        // default (launch) dir if the session is somehow missing from state.
        let cwd = {
            let state = self.state.read();
            state.session.get(&payload.session_id).map_or_else(
                || state.session.default_cwd().clone(),
                |s| s.cwd().to_path_buf(),
            )
        };

        let (cancel_handle, handle) = match spawn_setup_command(&payload.command, &self.shell, &cwd)
        {
            Ok(pair) => pair,
            Err(e) => {
                // Failed to even start the command.
                let error_msg = format!("Failed to start setup command: {e}");
                {
                    let mut state = self.state.write();
                    if let Some(session) = state.session.get_mut(&payload.session_id) {
                        session.complete_busy();
                    }
                }
                let existing_cwd = self
                    .state
                    .read()
                    .session
                    .get(&payload.session_id)
                    .map(|s| s.cwd().to_path_buf())
                    .unwrap_or_default();
                self.publish(PushChatEntry {
                    session_id: payload.session_id.clone(),
                    entry: ChatEntry::error(&error_msg),
                })
                .await;
                self.publish(SessionSetupCompleted {
                    session_id: payload.session_id.clone(),
                    cwd: existing_cwd,
                    error: Some(error_msg),
                })
                .await;
                return;
            }
        };

        let session_id = payload.session_id.clone();

        let bus = self.bus().clone();
        let _handle = tokio::spawn(async move {
            let result = handle.await;
            let (cwd, error) = match result {
                Ok(Ok(cwd)) => (cwd, None as Option<String>),
                Ok(Err(report)) => {
                    let error_msg =
                        if let Some(cmd_err) = report.downcast_ref::<crate::feat::session_lifecycle::command_runner::LifecycleCommandError>() {
                            crate::feat::session::session_actor::handlers::lifecycle::format_lifecycle_error(cmd_err)
                        } else {
                            crate::feat::session::session_actor::handlers::lifecycle::strip_ansi(&format!("{report:#?}"))
                        };
                    (None, Some(error_msg))
                }
                Err(_) => {
                    // Task was cancelled (abort).
                    (None, Some("Setup command was cancelled".to_owned()))
                }
            };
            bus.publish(FinishSessionSetup {
                session_id,
                cwd,
                error,
            })
            .await;
        });

        self.lifecycle_child = Some(cancel_handle);
    }

    /// Handle `FinishSessionSetup` - completion of an async setup shell command.
    ///
    /// Called by the spawned tokio task after the setup shell command finishes.
    /// Clears the lifecycle child, completes busy, sets CWD,
    /// advances lifecycle state, and emits events.
    pub(in crate::feat::session::session_actor) async fn handle_finish_session_setup(
        &mut self,
        payload: &crate::feat::session_lifecycle::protocol::command::FinishSessionSetup,
    ) {
        // Clear lifecycle child handle.
        self.lifecycle_child = None;

        // Complete busy.
        {
            let mut state = self.state.write();
            if let Some(session) = state.session.get_mut(&payload.session_id) {
                session.complete_busy();
            }
        }

        match (&payload.cwd, &payload.error) {
            (Some(cwd), None) => {
                // Success.
                {
                    let mut state = self.state.write();
                    if let Some(session) = state.session.get_mut(&payload.session_id) {
                        session.set_cwd(cwd.clone());
                        session.advance_lifecycle_after_setup();
                    }
                }

                self.publish(PushChatEntry {
                    session_id: payload.session_id.clone(),
                    entry: setup_complete_msg(cwd),
                })
                .await;

                self.publish(SessionSetupCompleted {
                    session_id: payload.session_id.clone(),
                    cwd: cwd.clone(),
                    error: None,
                })
                .await;
            }
            (_, Some(error_msg)) => {
                // Error.
                let existing_cwd = {
                    let state = self.state.read();
                    // Preserve the session's inherited CWD; do not overwrite with
                    // the app launch dir.
                    state.session.get(&payload.session_id).map_or_else(
                        || state.session.default_cwd().clone(),
                        |s| s.cwd().to_path_buf(),
                    )
                };

                let entry = ChatEntry::error(error_msg);

                self.publish(PushChatEntry {
                    session_id: payload.session_id.clone(),
                    entry,
                })
                .await;

                self.publish(SessionSetupCompleted {
                    session_id: payload.session_id.clone(),
                    cwd: existing_cwd,
                    error: Some(error_msg.clone()),
                })
                .await;
            }
            (None, None) => {
                // Success without a CWD - side-effect-only setup (no stdout output).
                // Keep the inherited CWD, advance lifecycle so teardown-on-close
                // fires, and surface an informational note so the user knows no
                // path was returned.
                let existing_cwd = {
                    let mut state = self.state.write();
                    if let Some(session) = state.session.get_mut(&payload.session_id) {
                        session.advance_lifecycle_after_setup();
                    }
                    state.session.get(&payload.session_id).map_or_else(
                        || state.session.default_cwd().clone(),
                        |s| s.cwd().to_path_buf(),
                    )
                };
                self.publish(PushChatEntry {
                    session_id: payload.session_id.clone(),
                    entry: no_output_info(&existing_cwd),
                })
                .await;
                self.publish(SessionSetupCompleted {
                    session_id: payload.session_id.clone(),
                    cwd: existing_cwd,
                    error: None,
                })
                .await;
            }
        }
    }

    /// Runs a builtin lifecycle setup by looking up the handler in the registry.
    async fn run_builtin_setup(
        &self,
        session_id: &crate::protocol::SessionId,
        id: &crate::feat::session_lifecycle::builtin::BuiltinId,
        args: &[String],
    ) {
        let Some(handler) = self.builtin_registry.get(id) else {
            let error_msg = format!("unknown builtin lifecycle: {id}");
            tracing::error!(%id, "builtin handler not found in registry");

            let existing_cwd = {
                let state = self.state.read();
                // Preserve the session's inherited CWD; do not overwrite with
                // the app launch dir.
                state.session.get(session_id).map_or_else(
                    || state.session.default_cwd().clone(),
                    |s| s.cwd().to_path_buf(),
                )
            };

            self.publish(PushChatEntry {
                session_id: session_id.clone(),
                entry: ChatEntry::error(&error_msg),
            })
            .await;

            self.publish(SessionSetupCompleted {
                session_id: session_id.clone(),
                cwd: existing_cwd,
                error: Some(error_msg),
            })
            .await;
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

                self.publish(PushChatEntry {
                    session_id: session_id.clone(),
                    entry: setup_complete_msg(&cwd),
                })
                .await;

                self.publish(SessionSetupCompleted {
                    session_id: session_id.clone(),
                    cwd,
                    error: None,
                })
                .await;
            }
            Err(report) => {
                let error_msg = format!("builtin setup failed: {report:#?}");
                let existing_cwd = {
                    let state = self.state.read();
                    // Preserve the session's inherited CWD; do not overwrite with
                    // the app launch dir.
                    state.session.get(session_id).map_or_else(
                        || state.session.default_cwd().clone(),
                        |s| s.cwd().to_path_buf(),
                    )
                };

                self.publish(PushChatEntry {
                    session_id: session_id.clone(),
                    entry: ChatEntry::error(&error_msg),
                })
                .await;

                self.publish(SessionSetupCompleted {
                    session_id: session_id.clone(),
                    cwd: existing_cwd,
                    error: Some(error_msg),
                })
                .await;
            }
        }
    }

    /// RunSessionTeardown: teardown-only handler (`t` key).
    ///
    /// For shell teardowns: sets `Working` phase, spawns a tokio task
    /// to run the shell command, and returns immediately. The spawned task
    /// sends `FinishSessionTeardown` back when complete.
    ///
    /// For builtin teardowns: runs inline (synchronous, no blocking).
    #[expect(
        clippy::too_many_lines,
        clippy::items_after_statements,
        reason = "handler reads best as a single unit"
    )]
    pub(in crate::feat::session::session_actor) async fn handle_run_session_teardown(
        &mut self,
        payload: &RunSessionTeardown,
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
                // Mark session as busy.
                let rendered = {
                    let mut state = self.state.write();
                    let Some(session) = state.session.get_mut(&payload.session_id) else {
                        return;
                    };
                    session.begin_busy();
                    use crate::feat::session_lifecycle::command_template::CommandTemplate;
                    let template = CommandTemplate::parse(shell_cmd);

                    if lifecycle_args.is_empty() {
                        shell_cmd.clone()
                    } else {
                        template.render(&lifecycle_args)
                    }
                };

                // Push "running" entry.

                self.push_and_save(&payload.session_id, teardown_running_msg())
                    .await;

                // Spawn tokio task to run the shell command.
                let session_id = payload.session_id.clone();
                let shell = self.shell.clone();

                // Read the session's CWD so the teardown script runs in the
                // inherited session dir, not jinn's process dir.
                let cwd = {
                    let state = self.state.read();
                    state.session.get(&payload.session_id).map_or_else(
                        || state.session.default_cwd().clone(),
                        |s| s.cwd().to_path_buf(),
                    )
                };
                let spawn_result =
                    crate::feat::session_lifecycle::command_runner::spawn_teardown_command(
                        &rendered, &shell, &cwd,
                    );

                match spawn_result {
                    Ok((cancel_handle, join_handle)) => {
                        self.lifecycle_child = Some(cancel_handle);

                        let bus = self.bus().clone();
                        let _handle = tokio::spawn(async move {
                            let result = join_handle.await;
                            let error = match result {
                                Ok(Ok(())) => None,
                                Ok(Err(report)) => {
                                    if let Some(cmd_err) =
                                        report.downcast_ref::<crate::feat::session_lifecycle::command_runner::LifecycleCommandError>()
                                    {
                                        Some(crate::feat::session::session_actor::handlers::lifecycle::format_lifecycle_error(cmd_err))
                                    } else {
                                        Some(crate::feat::session::session_actor::handlers::lifecycle::strip_ansi(&format!(
                                            "{report:#?}"
                                        )))
                                    }
                                }
                                Err(_) => Some("Teardown command was cancelled".to_owned()),
                            };
                            bus.publish(FinishSessionTeardown {
                                session_id,
                                close_after: false,
                                error,
                            })
                            .await;
                        });
                    }
                    Err(report) => {
                        let error_msg = format!("Failed to start teardown command: {report}");
                        self.publish(FinishSessionTeardown {
                            session_id,
                            close_after: false,
                            error: Some(error_msg),
                        })
                        .await;
                    }
                }
            }
            crate::feat::session_lifecycle::builtin::LifecycleCommand::Builtin(id) => {
                // Builtin teardown is synchronous - run inline.
                let success = self
                    .run_builtin_teardown(&payload.session_id, id, &lifecycle_args)
                    .await;

                if !success {
                    self.publish(SessionTeardownFinished {
                        session_id: payload.session_id.clone(),
                        error: Some("teardown failed".to_owned()),
                    })
                    .await;
                    return;
                }

                // Push success entry via PushChatEntry (persists automatically).
                self.publish(PushChatEntry {
                    session_id: payload.session_id.clone(),
                    entry: teardown_success_msg(),
                })
                .await;

                self.publish(SessionTeardownFinished {
                    session_id: payload.session_id.clone(),
                    error: None,
                })
                .await;
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
    ) -> bool {
        let Some(handler) = self.builtin_registry.get(id) else {
            let error_msg = format!("unknown builtin lifecycle: {id}");
            tracing::error!(%id, "builtin handler not found in registry for teardown");

            self.publish(PushChatEntry {
                session_id: session_id.clone(),
                entry: ChatEntry::error(&error_msg),
            })
            .await;
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
            self.publish(PushChatEntry {
                session_id: session_id.clone(),
                entry: ChatEntry::error(&error_msg),
            })
            .await;
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
    #[expect(clippy::too_many_lines, reason = "handler reads best as a single unit")]
    pub(in crate::feat::session::session_actor) async fn handle_close_session(
        &mut self,
        payload: &CloseSession,
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
                        // For shell teardowns: mark busy, spawn tokio task,
                        // then return immediately. The spawned task signals completion
                        // via FinishSessionTeardown with close_after: true.
                        let rendered = {
                            use crate::feat::session_lifecycle::command_template::CommandTemplate;

                            let mut state = self.state.write();
                            let Some(session) = state.session.get_mut(&payload.session_id) else {
                                return;
                            };
                            session.begin_busy();
                            let template = CommandTemplate::parse(&shell_cmd);

                            if lifecycle_args.is_empty() {
                                shell_cmd.clone()
                            } else {
                                template.render(&lifecycle_args)
                            }
                        };

                        self.push_and_save(&payload.session_id, teardown_running_msg())
                            .await;

                        let session_id = payload.session_id.clone();
                        let shell = self.shell.clone();

                        // Read the session's CWD so the close-teardown script runs
                        // in the inherited session dir, not jinn's process dir.
                        let cwd = {
                            let state = self.state.read();
                            state.session.get(&payload.session_id).map_or_else(
                                || state.session.default_cwd().clone(),
                                |s| s.cwd().to_path_buf(),
                            )
                        };

                        let spawn_result =
                            crate::feat::session_lifecycle::command_runner::spawn_teardown_command(
                                &rendered, &shell, &cwd,
                            );

                        match spawn_result {
                            Ok((cancel_handle, join_handle)) => {
                                self.lifecycle_child = Some(cancel_handle);
                                let bus = self.bus().clone();
                                let _handle = tokio::spawn(async move {
                                    let result = join_handle.await;
                                    let error = match result {
                                        Ok(Ok(())) => None,
                                        Ok(Err(report)) => {
                                            if let Some(cmd_err) =
                                                report.downcast_ref::<crate::feat::session_lifecycle::command_runner::LifecycleCommandError>()
                                            {
                                                Some(crate::feat::session::session_actor::handlers::lifecycle::format_lifecycle_error(cmd_err))
                                            } else {
                                                Some(crate::feat::session::session_actor::handlers::lifecycle::strip_ansi(&format!(
                                                    "{report:#?}"
                                                )))
                                            }
                                        }
                                        Err(_) => Some("Teardown command was cancelled".to_owned()),
                                    };
                                    bus.publish(FinishSessionTeardown {
                                        session_id,
                                        close_after: true,
                                        error,
                                    })
                                    .await;
                                });
                            }
                            Err(report) => {
                                let error_msg =
                                    format!("Failed to start teardown command: {report}");
                                self.publish(FinishSessionTeardown {
                                    session_id,
                                    close_after: true,
                                    error: Some(error_msg),
                                })
                                .await;
                            }
                        }

                        return; // Return immediately - async result handled via FinishSessionTeardown
                    }
                    crate::feat::session_lifecycle::builtin::LifecycleCommand::Builtin(id) => {
                        let success = self
                            .run_builtin_teardown(&payload.session_id, &id, &lifecycle_args)
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

        // Step 2b: Snapshot stats before removing from memory.
        {
            let state = self.state.read();
            if let Some(session) = state.session.get(&payload.session_id) {
                let frozen = crate::feat::session::snapshot_frozen_node(session);
                drop(state);
                self.state.write().session.insert_frozen_node(frozen);
            }
        }

        // Step 3: Remove from memory.
        self.remove_and_replace(&payload.session_id);

        // Step 4: Notify.
        self.publish(SessionArchived {
            session_id: payload.session_id.clone(),
        })
        .await;

        self.publish(SessionClosed {
            session_id: payload.session_id.clone(),
        })
        .await;
    }

    /// Handle `FinishSessionTeardown` - completion of an async teardown shell command.
    ///
    /// Called by the spawned tokio task after the teardown shell command finishes.
    /// Depending on `payload.close_after`:
    /// - `false` (teardown-only, `t` key): advance lifecycle state, persist, emit events
    /// - `true` (close-with-teardown, `x` key): archive and remove the session
    ///
    /// On error, an error entry is pushed and the session returns to `Idle` phase.
    pub(in crate::feat::session::session_actor) async fn handle_finish_session_teardown(
        &mut self,
        payload: &crate::feat::session_lifecycle::protocol::command::FinishSessionTeardown,
    ) {
        use crate::feat::session::chat_session::SessionState;

        // Clear lifecycle child handle.
        self.lifecycle_child = None;

        // Complete busy.
        {
            let mut state = self.state.write();
            let Some(session) = state.session.get_mut(&payload.session_id) else {
                return;
            };
            session.complete_busy();
        }

        if let Some(ref error_msg) = payload.error {
            // Teardown failed - push error entry, emit failure, cancel working.
            self.publish(PushChatEntry {
                session_id: payload.session_id.clone(),
                entry: ChatEntry::error(format!("Teardown failed: {error_msg}")),
            })
            .await;

            self.publish(SessionTeardownFinished {
                session_id: payload.session_id.clone(),
                error: Some(error_msg.clone()),
            })
            .await;

            // Busy count already decremented by complete_busy(). No phase change to emit.
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

            // Snapshot stats before removing from memory.
            {
                let state = self.state.read();
                if let Some(session) = state.session.get(&payload.session_id) {
                    let frozen = crate::feat::session::snapshot_frozen_node(session);
                    drop(state);
                    self.state.write().session.insert_frozen_node(frozen);
                }
            }

            self.remove_and_replace(&payload.session_id);

            // Emit events.
            self.publish(SessionArchived {
                session_id: payload.session_id.clone(),
            })
            .await;
            self.publish(SessionClosed {
                session_id: payload.session_id.clone(),
            })
            .await;
            self.publish(SessionTeardownFinished {
                session_id: payload.session_id.clone(),
                error: None,
            })
            .await;
        } else {
            // Teardown-only: advance lifecycle, persist, push success entry, emit.
            {
                let mut state = self.state.write();
                let Some(session) = state.session.get_mut(&payload.session_id) else {
                    return;
                };
                session.advance_lifecycle_after_teardown();
            }

            // Persist the lifecycle state change.
            self.save_active_session(&payload.session_id).await;

            // Push success entry.
            self.publish(PushChatEntry {
                session_id: payload.session_id.clone(),
                entry: teardown_success_msg(),
            })
            .await;

            // Emit completion event.
            self.publish(SessionTeardownFinished {
                session_id: payload.session_id.clone(),
                error: None,
            })
            .await;
        }
    }

    /// Handle `CancelLifecycleCommand` - terminate a running lifecycle process.
    ///
    /// Per Option B, the cancel handler does only the kill + abort. All cleanup
    /// (busy decrement, chat entry, phase transition, cwd) is owned by the
    /// `FinishSessionSetup` / `FinishSessionTeardown` handlers, which fire when
    /// the aborted reader task's wrapper observes the resulting `JoinError` and
    /// takes its existing "... was cancelled" branch.
    ///
    /// This function is lock-free and await-free: it signals the process group by
    /// PID and aborts the reader task handle. Both are safe to call from a runtime
    /// worker thread, which is why this replaced the old `blocking_lock` path that
    /// panicked on `Cannot block the current thread from within a runtime`.
    pub(in crate::feat::session::session_actor) fn handle_cancel_lifecycle_command(
        &mut self,
        _payload: &crate::feat::session_lifecycle::protocol::command::CancelLifecycleCommand,
    ) {
        if let Some(handle) = self.lifecycle_child.take() {
            // Kill the process group by PID (instant SIGKILL; reaches
            // backgrounded descendants via the group signal).
            crate::common::process_kill::kill_process_group_by_pid(handle.pid);
            // Abort the INNER reader task. Its outer wrapper (spawned in
            // `handle_setup_lifecycle_command` / the teardown equivalents)
            // observes the resulting `JoinError` on its `handle.await` and takes
            // the `Err(_) => "... was cancelled"` arm, which sends
            // `FinishSessionSetup` / `FinishSessionTeardown` for cleanup.
            handle.abort_handle.abort();
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
    /// put away - it can be unarchived later.
    pub(in crate::feat::session::session_actor) async fn handle_archive_session(
        &self,
        payload: &ArchiveSession,
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

        // Step 2: Snapshot stats before removing from memory.
        {
            let state = self.state.read();
            if let Some(session) = state.session.get(&payload.session_id) {
                let frozen = crate::feat::session::snapshot_frozen_node(session);
                drop(state);
                self.state.write().session.insert_frozen_node(frozen);
            }
        }

        // Step 3: Remove from memory.
        self.remove_and_replace(&payload.session_id);

        // Step 3: Notify.
        self.publish(SessionArchived {
            session_id: payload.session_id.clone(),
        })
        .await;

        self.publish(SessionClosed {
            session_id: payload.session_id.clone(),
        })
        .await;
    }

    /// Remove session from HashMap, create replacement if empty, reconcile cursor.
    ///
    /// Pure state mutation helper. Does NOT emit events - callers handle notifications.
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
            &mut state, session_id,
        );

        let app_state = self.services.app_state_storage.read();
        let fresh_session = {
            let model = app_state.last_model.unwrap_or_default();

            ChatSessionState::new_with_profile(
                crate::feat::session::profile::SessionProfile::from_model_selection(model),
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

    /// Handle `SetSessionCwd` - set a session's cwd and broadcast the change.
    ///
    /// Writes the cwd onto the session in state, then emits `SessionCwdChanged`
    /// so subscribed discovery scan actors re-scan skills, prompts, and context
    /// files for the new cwd.
    pub(in crate::feat::session::session_actor) async fn handle_set_session_cwd(
        &self,
        payload: &SetSessionCwd,
    ) {
        {
            let mut state = self.state.write();
            if let Some(session) = state.session.get_mut(&payload.session_id) {
                session.set_cwd(payload.cwd.clone());
            }
        }
        self.publish(SessionCwdChanged {
            session_id: payload.session_id.clone(),
            cwd: payload.cwd.clone(),
        })
        .await;
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        clippy::similar_names,
        reason = "test code"
    )]
    use super::super::super::helpers::{
        test_actor, test_actor_recording, test_actor_with_store_recording,
    };
    use super::{
        no_output_info, setup_complete_msg, setup_running_msg, strip_ansi, teardown_running_msg,
    };

    use crate::feat::chat_input::protocol::command::PushChatEntry;
    use crate::feat::session::chat_session::ChatSessionState;
    use crate::feat::session::protocol::close_session::CloseSession;
    use crate::feat::session::protocol::session_archived::SessionArchived;
    use crate::feat::session::protocol::session_closed::SessionClosed;
    use crate::feat::session_lifecycle::protocol::command::{
        CancelLifecycleCommand, FinishSessionSetup, FinishSessionTeardown, RunSessionSetup,
        RunSessionTeardown, SetSessionCwd,
    };
    use crate::feat::session_lifecycle::protocol::event::{
        SessionCwdChanged, SessionTeardownFinished,
    };
    use crate::protocol::{ChatEntry, ChatEntryKind, SessionId};
    use std::path::Path;

    #[rstest::rstest]
    #[tokio::test]
    async fn strip_ansi_removes_bold_codes() {
        let input = "\x1b[1mbold text\x1b[22m";
        let result = strip_ansi(input);
        assert_eq!(result, "bold text");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn strip_ansi_removes_color_codes() {
        let input = "\x1b[31mred\x1b[0m";
        let result = strip_ansi(input);
        assert_eq!(result, "red");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn strip_ansi_passes_plain_text() {
        let result = strip_ansi("hello world");
        assert_eq!(result, "hello world");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn strip_ansi_handles_chained_codes() {
        let result = strip_ansi("\x1b[1m\x1b[31mbold red\x1b[0m");
        assert_eq!(result, "bold red");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn strip_ansi_handles_complex_csi_sequences() {
        let result = strip_ansi("\x1b[38;5;196mcolored\x1b[0m");
        assert_eq!(result, "colored");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn strip_ansi_handles_empty_string() {
        assert_eq!(strip_ansi(""), "");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn strip_ansi_handles_text_with_no_ansi() {
        let input = "normal text\nwith newlines";
        assert_eq!(strip_ansi(input), input);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn no_output_info_is_system_entry_with_cwd() {
        let cwd = Path::new("/tmp/test-project");
        let entry = no_output_info(cwd);
        let ChatEntryKind::System(text) = &entry.kind else {
            panic!("expected System entry, got {:?}", entry.kind);
        };
        assert!(text.contains("No path returned by setup command"));
        assert!(text.contains("/tmp/test-project"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn setup_running_msg_is_system_with_gear_emoji() {
        let entry = setup_running_msg();
        let ChatEntryKind::System(text) = &entry.kind else {
            panic!("expected System entry, got {:?}", entry.kind);
        };
        assert!(text.contains("\u{2699}\u{FE0F}"));
        assert!(text.contains("Running setup script"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn setup_complete_msg_is_system_with_checkmark_and_cwd() {
        let cwd = Path::new("/tmp/my-project");
        let entry = setup_complete_msg(cwd);
        let ChatEntryKind::System(text) = &entry.kind else {
            panic!("expected System entry, got {:?}", entry.kind);
        };
        assert!(text.contains("\u{2705}"));
        assert!(text.contains("Setup complete"));
        assert!(text.contains("/tmp/my-project"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn teardown_running_msg_is_system_with_gear_emoji() {
        let entry = teardown_running_msg();
        let ChatEntryKind::System(text) = &entry.kind else {
            panic!("expected System entry, got {:?}", entry.kind);
        };
        assert!(text.contains("\u{2699}\u{FE0F}"));
        assert!(text.contains("Running teardown script"));
    }

    #[tokio::test]
    async fn teardown_failure_does_not_switch_active_session() {
        use crate::feat::preferences_actor::user_preferences::SessionLifecycle;

        let (mut actor, audit) = test_actor_recording().await;
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
            .handle_run_session_teardown(&RunSessionTeardown {
                session_id: target_id.clone(),
                command: "exit 1".to_owned(),
                args: vec![],
            })
            .await;

        // Simulate async teardown failure.
        let finish = FinishSessionTeardown {
            session_id: target_id.clone(),
            close_after: false,
            error: Some("teardown failed".to_owned()),
        };
        actor.handle_finish_session_teardown(&finish).await;

        // Then the active session is unchanged.
        let state = actor.state.read();
        assert_eq!(*state.session.active_session_id(), original_active);
        drop(state);

        // And SessionTeardownFinished was emitted with error.
        let found = audit
            .of_type::<SessionTeardownFinished>()
            .iter()
            .any(|e| e.session_id == target_id && e.error.is_some());
        assert!(found, "expected SessionTeardownFinished event");
    }

    #[tokio::test]
    async fn teardown_failure_does_not_push_input_scope() {
        use crate::feat::preferences_actor::user_preferences::SessionLifecycle;

        let mut actor = test_actor().await;
        let session_id = {
            let mut state = actor.state.write();
            state.frontend.scope_stack.clear_overlays();
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
            .handle_run_session_teardown(&RunSessionTeardown {
                session_id,
                command: "exit 1".to_owned(),
                args: vec![],
            })
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
        use crate::feat::preferences_actor::user_preferences::SessionLifecycle;

        let (mut actor, audit) = test_actor_recording().await;
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

        actor
            .handle_run_session_teardown(&RunSessionTeardown {
                session_id: session_id.clone(),
                command: "exit 1".to_owned(),
                args: vec![],
            })
            .await;

        let finish = FinishSessionTeardown {
            session_id: session_id.clone(),
            close_after: false,
            error: Some("exit code 1".to_owned()),
        };
        actor.handle_finish_session_teardown(&finish).await;

        let has_error = audit.of_type::<PushChatEntry>().iter().any(
            |e| matches!(&e.entry.kind, ChatEntryKind::Error(msg) if msg.contains("exit code")),
        );
        assert!(has_error, "expected PushChatEntry command with error entry");
    }

    #[tokio::test]
    async fn teardown_failure_emits_session_teardown_completed_with_error() {
        use crate::feat::preferences_actor::user_preferences::SessionLifecycle;

        let (mut actor, audit) = test_actor_recording().await;
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

        actor
            .handle_run_session_teardown(&RunSessionTeardown {
                session_id: session_id.clone(),
                command: "exit 1".to_owned(),
                args: vec![],
            })
            .await;

        let finish = FinishSessionTeardown {
            session_id: session_id.clone(),
            close_after: false,
            error: Some("teardown failed".to_owned()),
        };
        actor.handle_finish_session_teardown(&finish).await;

        let found = audit
            .of_type::<SessionTeardownFinished>()
            .iter()
            .any(|e| e.session_id == session_id && e.error.is_some());
        assert!(found, "expected SessionTeardownFinished with error");
    }

    #[tokio::test]
    async fn remove_session_removes_session_from_hashmap() {
        let (mut actor, audit) = test_actor_recording().await;
        let second = ChatSessionState::new();
        let second_id = second.session_id().clone();
        {
            let mut state = actor.state.write();
            state.session.insert(second);
        }

        actor
            .handle_close_session(&CloseSession {
                session_id: second_id.clone(),
            })
            .await;

        let state = actor.state.read();
        assert!(!state.session.contains(&second_id));
        assert_eq!(state.session.session_count(), 1);
        drop(state);

        let found = audit
            .of_type::<SessionClosed>()
            .iter()
            .any(|e| e.session_id == second_id);
        assert!(found, "expected SessionClosed event");
    }

    #[tokio::test]
    async fn remove_session_creates_new_session_when_last_removed() {
        let (mut actor, audit) = test_actor_recording().await;
        let only_id = actor.state.read().session.active_session_id().clone();

        actor
            .handle_close_session(&CloseSession {
                session_id: only_id.clone(),
            })
            .await;

        let state = actor.state.read();
        assert!(!state.session.contains(&only_id));
        assert_eq!(state.session.session_count(), 1);
        assert_ne!(*state.session.active_session_id(), only_id);
        drop(state);

        let found = audit
            .of_type::<SessionClosed>()
            .iter()
            .any(|e| e.session_id == only_id);
        assert!(found, "expected SessionClosed event");
    }

    #[tokio::test]
    async fn remove_session_switches_active_when_active_is_removed() {
        let mut actor = test_actor().await;
        let second = ChatSessionState::new();
        let second_id = second.session_id().clone();
        {
            let mut state = actor.state.write();
            state.session.insert(second);
            state.session.set_active(second_id.clone());
        }

        actor
            .handle_close_session(&CloseSession {
                session_id: second_id.clone(),
            })
            .await;

        let state = actor.state.read();
        assert_ne!(*state.session.active_session_id(), second_id);
        assert_eq!(state.session.session_count(), 1);
    }

    #[tokio::test]
    async fn remove_session_emits_session_removed_event() {
        let (mut actor, audit) = test_actor_recording().await;
        let second = ChatSessionState::new();
        let second_id = second.session_id().clone();
        {
            let mut state = actor.state.write();
            state.session.insert(second);
        }

        actor
            .handle_close_session(&CloseSession {
                session_id: second_id.clone(),
            })
            .await;

        let count = audit
            .of_type::<SessionClosed>()
            .iter()
            .filter(|e| e.session_id == second_id)
            .count();
        assert_eq!(count, 1, "expected exactly one SessionClosed event");
    }

    #[tokio::test]
    async fn remove_session_is_noop_if_session_does_not_exist() {
        let (mut actor, audit) = test_actor_recording().await;
        let fake_id = SessionId::new();
        let original_len = actor.state.read().session.session_count();

        actor
            .handle_close_session(&CloseSession {
                session_id: fake_id.clone(),
            })
            .await;

        assert_eq!(actor.state.read().session.session_count(), original_len);

        let found = audit
            .of_type::<SessionClosed>()
            .iter()
            .any(|e| e.session_id == fake_id);
        assert!(
            !found,
            "did not expect SessionClosed for nonexistent session"
        );
    }

    #[tokio::test]
    async fn teardown_only_success_does_not_remove_session() {
        let mut actor = test_actor().await;
        let session_id = actor.state.read().session.active_session_id().clone();
        let original_count = actor.state.read().session.session_count();

        actor
            .handle_run_session_teardown(&RunSessionTeardown {
                session_id: session_id.clone(),
                command: "echo test".to_owned(),
                args: vec![],
            })
            .await;

        let state = actor.state.read();
        assert!(state.session.contains(&session_id));
        assert_eq!(state.session.session_count(), original_count);
    }

    #[tokio::test]
    async fn teardown_only_success_does_not_emit_session_removed() {
        let (mut actor, audit) = test_actor_recording().await;
        let session_id = actor.state.read().session.active_session_id().clone();

        actor
            .handle_run_session_teardown(&RunSessionTeardown {
                session_id: session_id.clone(),
                command: "echo test".to_owned(),
                args: vec![],
            })
            .await;

        let found = audit
            .of_type::<SessionClosed>()
            .iter()
            .any(|e| e.session_id == session_id);
        assert!(!found, "did not expect SessionClosed for teardown-only");
    }

    #[tokio::test]
    async fn teardown_only_success_emits_session_teardown_completed() {
        use crate::feat::preferences_actor::user_preferences::SessionLifecycle;

        let (mut actor, audit) = test_actor_recording().await;
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

        actor
            .handle_run_session_teardown(&RunSessionTeardown {
                session_id: session_id.clone(),
                command: "echo test".to_owned(),
                args: vec![],
            })
            .await;

        let finish = FinishSessionTeardown {
            session_id: session_id.clone(),
            close_after: false,
            error: None,
        };
        actor.handle_finish_session_teardown(&finish).await;

        let found = audit
            .of_type::<SessionTeardownFinished>()
            .iter()
            .any(|e| e.session_id == session_id && e.error.is_none());
        assert!(found, "expected SessionTeardownFinished event");
    }

    #[tokio::test]
    async fn teardown_only_success_emits_push_chat_entry() {
        use crate::feat::preferences_actor::user_preferences::SessionLifecycle;

        let (mut actor, audit) = test_actor_recording().await;
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

        actor
            .handle_run_session_teardown(&RunSessionTeardown {
                session_id: session_id.clone(),
                command: "echo test".to_owned(),
                args: vec![],
            })
            .await;

        let finish = FinishSessionTeardown {
            session_id: session_id.clone(),
            close_after: false,
            error: None,
        };
        actor.handle_finish_session_teardown(&finish).await;

        let has_success = audit
            .of_type::<PushChatEntry>()
            .iter()
            .any(|e| matches!(&e.entry.kind, ChatEntryKind::System(t) if t.contains("Teardown")));
        assert!(
            has_success,
            "expected PushChatEntry with teardown success entry"
        );
    }

    #[tokio::test]
    async fn close_session_with_nothing_ran_skips_teardown_and_archives() {
        let (mut actor, audit) = test_actor_recording().await;
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

        actor
            .handle_close_session(&CloseSession {
                session_id: session_id.clone(),
            })
            .await;

        let state = actor.state.read();
        assert!(!state.session.contains(&session_id));

        let has_teardown = !audit.of_type::<SessionTeardownFinished>().is_empty();
        assert!(
            !has_teardown,
            "did not expect SessionTeardownFinished for NothingRan"
        );

        let has_archived = !audit.of_type::<SessionArchived>().is_empty();
        assert!(has_archived, "expected SessionArchived");
    }

    #[tokio::test]
    async fn archive_session_without_lifecycle_removes_from_memory() {
        let (actor, audit) = test_actor_recording().await;
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

        actor
            .handle_archive_session(
                &crate::feat::session::protocol::archive_session::ArchiveSession {
                    session_id: target_id.clone(),
                },
            )
            .await;

        let state = actor.state.read();
        assert!(!state.session.contains(&target_id));
        assert_eq!(state.session.session_count(), 1);
        drop(state);

        let has_archived = audit
            .of_type::<SessionArchived>()
            .iter()
            .any(|e| e.session_id == target_id);
        assert!(has_archived, "expected SessionArchived");

        let has_closed = audit
            .of_type::<SessionClosed>()
            .iter()
            .any(|e| e.session_id == target_id);
        assert!(has_closed, "expected SessionClosed");

        let has_teardown = !audit.of_type::<SessionTeardownFinished>().is_empty();
        assert!(
            !has_teardown,
            "did not expect SessionTeardownFinished for archive"
        );
    }

    #[tokio::test]
    async fn archive_empty_session_removes_and_archives() {
        let (actor, audit) = test_actor_recording().await;
        let second = ChatSessionState::new();
        let _second_id = second.session_id().clone();
        let target_id = {
            let mut state = actor.state.write();
            state.session.insert(second);
            state.session.active_session_id().clone()
        };

        actor
            .handle_archive_session(
                &crate::feat::session::protocol::archive_session::ArchiveSession {
                    session_id: target_id.clone(),
                },
            )
            .await;

        assert!(!actor.state.read().session.contains(&target_id));

        let has_archived = !audit.of_type::<SessionArchived>().is_empty();
        assert!(has_archived, "expected SessionArchived for empty session");

        let has_closed = audit
            .of_type::<SessionClosed>()
            .iter()
            .any(|e| e.session_id == target_id);
        assert!(has_closed, "expected SessionClosed");
    }

    #[tokio::test]
    async fn archive_active_session_switches_to_next() {
        let actor = test_actor().await;
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

        actor
            .handle_archive_session(
                &crate::feat::session::protocol::archive_session::ArchiveSession {
                    session_id: active_id.clone(),
                },
            )
            .await;

        let state = actor.state.read();
        assert_ne!(*state.session.active_session_id(), active_id);
        assert_eq!(state.session.session_count(), 1);
    }

    #[tokio::test]
    async fn archive_last_session_creates_new_one() {
        let actor = test_actor().await;
        let only_id = {
            let mut state = actor.state.write();
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("msg"));
            state.session.active_session_id().clone()
        };

        actor
            .handle_archive_session(
                &crate::feat::session::protocol::archive_session::ArchiveSession {
                    session_id: only_id.clone(),
                },
            )
            .await;

        let state = actor.state.read();
        assert!(!state.session.contains(&only_id));
        assert_eq!(state.session.session_count(), 1);
        assert_ne!(*state.session.active_session_id(), only_id);
    }

    #[tokio::test]
    async fn close_session_leaves_lifecycle_at_setup_ran_when_teardown_fails() {
        use crate::feat::preferences_actor::user_preferences::SessionLifecycle;
        use crate::feat::session::chat_session::LifecycleScriptState;

        let mut actor = test_actor().await;
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

        actor
            .handle_close_session(&CloseSession {
                session_id: session_id.clone(),
            })
            .await;

        let finish = FinishSessionTeardown {
            session_id: session_id.clone(),
            close_after: true,
            error: Some("exit code 1".to_owned()),
        };
        actor.handle_finish_session_teardown(&finish).await;

        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert_eq!(
            session.lifecycle_script_state(),
            LifecycleScriptState::SetupRan
        );
        assert!(state.session.contains(&session_id));
    }

    #[tokio::test]
    async fn close_session_with_teardown_failure_pushes_error_entry() {
        use crate::feat::preferences_actor::user_preferences::SessionLifecycle;

        let (mut actor, audit) = test_actor_recording().await;
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

        actor
            .handle_close_session(&CloseSession {
                session_id: session_id.clone(),
            })
            .await;

        let finish = FinishSessionTeardown {
            session_id: session_id.clone(),
            close_after: true,
            error: Some("teardown failed".to_owned()),
        };
        actor.handle_finish_session_teardown(&finish).await;

        let has_error = audit
            .of_type::<PushChatEntry>()
            .iter()
            .any(|e| matches!(e.entry.kind, ChatEntryKind::Error(_)));
        assert!(has_error, "expected PushChatEntry command with error entry");
    }

    #[tokio::test]
    async fn close_session_advances_lifecycle_when_teardown_succeeds() {
        use crate::feat::preferences_actor::user_preferences::SessionLifecycle;

        let (mut actor, audit) = test_actor_recording().await;
        let second_session = ChatSessionState::new();
        let session_id = {
            let mut state = actor.state.write();
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

        actor
            .handle_close_session(&CloseSession {
                session_id: session_id.clone(),
            })
            .await;

        let finish = FinishSessionTeardown {
            session_id: session_id.clone(),
            close_after: true,
            error: None,
        };
        actor.handle_finish_session_teardown(&finish).await;

        assert!(!actor.state.read().session.contains(&session_id));

        let found = audit
            .of_type::<SessionTeardownFinished>()
            .iter()
            .any(|e| e.session_id == session_id && e.error.is_none());
        assert!(found, "expected SessionTeardownFinished with no error");
    }

    #[tokio::test]
    async fn archiving_empty_session_does_not_persist_non_interacted() {
        let (actor, store, _audit) = test_actor_with_store_recording(vec![]).await;
        let session_id = actor.state.read().session.active_session_id().clone();

        actor
            .handle_archive_session(
                &crate::feat::session::protocol::archive_session::ArchiveSession {
                    session_id: session_id.clone(),
                },
            )
            .await;

        assert!(
            store.last_saved_session(&session_id).is_none(),
            "empty non-interacted session should not be persisted"
        );
    }

    #[tokio::test]
    async fn closing_empty_session_does_not_persist_non_interacted() {
        let (mut actor, store, _audit) = test_actor_with_store_recording(vec![]).await;
        let session_id = actor.state.read().session.active_session_id().clone();

        actor
            .handle_close_session(&CloseSession {
                session_id: session_id.clone(),
            })
            .await;

        assert!(
            store.last_saved_session(&session_id).is_none(),
            "empty non-interacted session should not be persisted"
        );
    }

    #[tokio::test]
    async fn teardown_only_advances_lifecycle_to_teardown_ran() {
        use crate::feat::preferences_actor::user_preferences::SessionLifecycle;
        use crate::feat::session::chat_session::LifecycleScriptState;

        let mut actor = test_actor().await;
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

        actor
            .handle_run_session_teardown(&RunSessionTeardown {
                session_id: session_id.clone(),
                command: "exit 0".to_owned(),
                args: vec![],
            })
            .await;

        let finish = FinishSessionTeardown {
            session_id: session_id.clone(),
            close_after: false,
            error: None,
        };
        actor.handle_finish_session_teardown(&finish).await;

        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert_eq!(
            session.lifecycle_script_state(),
            LifecycleScriptState::TeardownRan
        );
        assert!(state.session.contains(&session_id));
    }

    #[tokio::test]
    async fn close_session_with_setup_ran_persists_teardown_ran() {
        use crate::feat::preferences_actor::user_preferences::SessionLifecycle;
        use crate::feat::session::chat_session::LifecycleScriptState;

        let (mut actor, store, _audit) = test_actor_with_store_recording(vec![]).await;
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

        actor
            .handle_close_session(&CloseSession {
                session_id: session_id.clone(),
            })
            .await;

        let finish = FinishSessionTeardown {
            session_id: session_id.clone(),
            close_after: true,
            error: None,
        };
        actor.handle_finish_session_teardown(&finish).await;

        let saved = store
            .last_saved_session(&session_id)
            .expect("session should have been saved");
        assert_eq!(
            saved.lifecycle_script_state(),
            LifecycleScriptState::TeardownRan
        );
    }

    #[tokio::test]
    async fn set_session_cwd_writes_cwd_onto_session() {
        use std::path::PathBuf;

        let actor = test_actor().await;
        let session_id = actor.state.read().session.active_session_id().clone();
        let new_cwd = PathBuf::from("/tmp/new-project");

        actor
            .handle_set_session_cwd(&SetSessionCwd {
                session_id: session_id.clone(),
                cwd: new_cwd.clone(),
            })
            .await;

        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert_eq!(session.cwd(), &new_cwd);
    }

    #[tokio::test]
    async fn set_session_cwd_emits_session_cwd_changed_event() {
        use std::path::PathBuf;

        let (actor, audit) = test_actor_recording().await;
        let session_id = actor.state.read().session.active_session_id().clone();
        let new_cwd = PathBuf::from("/tmp/other-project");

        actor
            .handle_set_session_cwd(&SetSessionCwd {
                session_id: session_id.clone(),
                cwd: new_cwd.clone(),
            })
            .await;

        let cwd_changed = audit.of_type::<SessionCwdChanged>();
        assert_eq!(cwd_changed.len(), 1);
        assert_eq!(cwd_changed[0].session_id, session_id);
        assert_eq!(cwd_changed[0].cwd, new_cwd);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn cancel_with_no_lifecycle_in_flight_is_noop() {
        let (mut actor, audit) = test_actor_recording().await;
        let payload = CancelLifecycleCommand {
            session_id: SessionId::new(),
        };

        actor.handle_cancel_lifecycle_command(&payload);

        assert!(audit.is_empty());
    }

    #[tokio::test]
    async fn finish_handler_owns_cleanup_after_cancel() {
        let (mut actor, audit) = test_actor_recording().await;
        let session_id = actor.state.read().session.active_session_id().clone();

        actor
            .handle_run_session_setup(&RunSessionSetup {
                session_id: session_id.clone(),
                command: "sleep 30".to_owned(),
                args: vec![],
                lifecycle_command: None,
            })
            .await;

        assert!(actor.lifecycle_child.is_some());
        assert_eq!(
            actor
                .state
                .read()
                .session
                .get(&session_id)
                .map(ChatSessionState::busy_count),
            Some(1)
        );

        actor.handle_cancel_lifecycle_command(&CancelLifecycleCommand {
            session_id: session_id.clone(),
        });

        assert!(actor.lifecycle_child.is_none());
        assert_eq!(
            actor
                .state
                .read()
                .session
                .get(&session_id)
                .map(ChatSessionState::busy_count),
            Some(1)
        );

        // Then the aborted reader task emits exactly one FinishSessionSetup.
        let finish = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let finishes = audit.of_type::<FinishSessionSetup>();
                if let Some(f) = finishes.into_iter().next() {
                    return f;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("FinishSessionSetup must be emitted after cancel");

        actor.handle_finish_session_setup(&finish).await;

        let push_count = audit.of_type::<PushChatEntry>().len();
        assert_eq!(push_count, 1);

        assert_eq!(
            actor
                .state
                .read()
                .session
                .get(&session_id)
                .map(ChatSessionState::busy_count),
            Some(0)
        );

        assert_eq!(
            actor
                .state
                .read()
                .session
                .get(&session_id)
                .map(ChatSessionState::phase),
            Some(crate::feat::session::phase_machine::PhaseKind::Idle)
        );
    }

    #[tokio::test]
    async fn setup_with_no_output_advances_to_setup_ran() {
        let (mut actor, audit) = test_actor_recording().await;
        let session_id = actor.state.read().session.active_session_id().clone();

        let finish = FinishSessionSetup {
            session_id: session_id.clone(),
            cwd: None,
            error: None,
        };
        actor.handle_finish_session_setup(&finish).await;

        assert_eq!(
            actor
                .state
                .read()
                .session
                .get(&session_id)
                .map(ChatSessionState::lifecycle_script_state),
            Some(crate::feat::session::chat_session::LifecycleScriptState::SetupRan)
        );

        let push_count = audit.of_type::<PushChatEntry>().len();
        assert_eq!(push_count, 1);
    }

    #[tokio::test]
    async fn no_output_fallback_preserves_inherited_session_cwd() {
        // Given an actor whose active session has a distinct (inherited) CWD
        // that differs from default_cwd().
        let (mut actor, _audit) = test_actor_recording().await;
        let session_id = actor.state.read().session.active_session_id().clone();
        let inherited_cwd = std::path::PathBuf::from("/tmp/inherited-project");
        {
            let mut state = actor.state.write();
            state
                .session
                .get_mut(&session_id)
                .unwrap()
                .set_cwd(inherited_cwd.clone());
        }
        assert_ne!(*actor.state.read().session.default_cwd(), inherited_cwd);

        // When the setup command finishes with no output (the (None, None)
        // fallback arm in handle_finish_session_setup).
        let finish = FinishSessionSetup {
            session_id: session_id.clone(),
            cwd: None,
            error: None,
        };
        actor.handle_finish_session_setup(&finish).await;

        // Then the session's CWD is the inherited value, not the app launch
        // dir (default_cwd).
        let cwd_after = actor
            .state
            .read()
            .session
            .get(&session_id)
            .map(|s| s.cwd().to_path_buf())
            .unwrap();
        assert_eq!(cwd_after, inherited_cwd);
    }

    #[tokio::test]
    async fn setup_success_overwrites_inherited_cwd_with_script_output() {
        // Given an actor whose active session has an inherited CWD.
        let (mut actor, _audit) = test_actor_recording().await;
        let session_id = actor.state.read().session.active_session_id().clone();
        let inherited_cwd = std::path::PathBuf::from("/tmp/inherited-project");
        {
            let mut state = actor.state.write();
            state
                .session
                .get_mut(&session_id)
                .unwrap()
                .set_cwd(inherited_cwd.clone());
        }

        // When the setup command finishes and echoes a different CWD
        // (the (Some(cwd), None) success arm in handle_finish_session_setup).
        let script_cwd = std::path::PathBuf::from("/tmp/script-output-dir");
        let finish = FinishSessionSetup {
            session_id: session_id.clone(),
            cwd: Some(script_cwd.clone()),
            error: None,
        };
        actor.handle_finish_session_setup(&finish).await;

        // Then the session's CWD is the script's output, not the inherited
        // value — the script-stdout-wins contract is preserved (AC3).
        let cwd_after = actor
            .state
            .read()
            .session
            .get(&session_id)
            .map(|s| s.cwd().to_path_buf())
            .unwrap();
        assert_eq!(cwd_after, script_cwd);
    }

    #[tokio::test]
    async fn setup_error_preserves_inherited_session_cwd() {
        // Given an actor whose active session has an inherited CWD.
        let (mut actor, _audit) = test_actor_recording().await;
        let session_id = actor.state.read().session.active_session_id().clone();
        let inherited_cwd = std::path::PathBuf::from("/tmp/inherited-project");
        {
            let mut state = actor.state.write();
            state
                .session
                .get_mut(&session_id)
                .unwrap()
                .set_cwd(inherited_cwd.clone());
        }

        // When the setup command finishes with an error (the
        // (_, Some(error_msg)) fallback arm in handle_finish_session_setup).
        let finish = FinishSessionSetup {
            session_id: session_id.clone(),
            cwd: None,
            error: Some("command failed: exit 1".to_owned()),
        };
        actor.handle_finish_session_setup(&finish).await;

        // Then the session's CWD is the inherited value, not the app launch
        // dir (default_cwd) — AC4.
        let cwd_after = actor
            .state
            .read()
            .session
            .get(&session_id)
            .map(|s| s.cwd().to_path_buf())
            .unwrap();
        assert_eq!(cwd_after, inherited_cwd);
    }

    #[tokio::test]
    async fn setup_spawn_falls_back_to_default_cwd_for_missing_session() {
        // Given an actor with no session matching the payload's session_id
        // (the defensive case: the spawn site must fall back to
        // default_cwd() and never panic / never pass a nonexistent path
        // to .current_dir()).
        let (mut actor, audit) = test_actor_recording().await;
        let missing_id = SessionId::new();

        // Sanity: the session really is absent from state.
        assert!(!actor.state.read().session.contains(&missing_id));

        // When firing RunSessionSetup for the missing session.
        actor
            .handle_run_session_setup(&RunSessionSetup {
                session_id: missing_id.clone(),
                command: "echo $PWD".to_owned(),
                args: vec![],
                lifecycle_command: None,
            })
            .await;

        // Then the spawn did not panic: the spawned task published
        // FinishSessionSetup with no error, proving the fallback CWD
        // (default_cwd) was a real, spawnable path rather than a
        // nonexistent one that would make .current_dir() fail.
        let finish = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let finishes = audit.of_type::<FinishSessionSetup>();
                if let Some(f) = finishes.into_iter().next() {
                    return f;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("FinishSessionSetup must be emitted even for a missing session");

        assert_eq!(finish.session_id, missing_id);
        assert!(
            finish.error.is_none(),
            "expected no spawn error for the default-cwd fallback, got {:?}",
            finish.error
        );
    }

    #[tokio::test]
    async fn teardown_spawn_runs_shell_script_in_session_cwd() {
        // Given an actor whose active session has an inherited CWD and a
        // teardown shell command that writes a marker file via a relative path.
        let dir = tempfile::tempdir().expect("tempdir");
        let (mut actor, audit) = test_actor_recording().await;
        let session_id = {
            use crate::feat::preferences_actor::user_preferences::SessionLifecycle;
            let mut state = actor.state.write();
            let session_id = state.session.active_session_id().clone();
            // Configure the session (CWD + lifecycle state + name) before touching
            // the preferences, so the two `state` borrows don't overlap.
            {
                let session = state.active_session_mut();
                session.set_cwd(dir.path().to_path_buf());
                // Teardown path only fires when the session is in SetupRan.
                session.advance_lifecycle_after_setup();
                session.set_lifecycle_name(Some("cwd-probe".to_owned()));
            }
            state.frontend.preferences.session_lifecycles = vec![SessionLifecycle {
                name: "cwd-probe".to_owned(),
                description: None,
                setup: None,
                teardown: Some(
                    crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(
                        "printf x > teardown-marker.log".to_owned(),
                    ),
                ),
            }];
            session_id
        };

        // When firing RunSessionTeardown (the `t` teardown-only path).
        actor
            .handle_run_session_teardown(&RunSessionTeardown {
                session_id: session_id.clone(),
                command: "printf x > teardown-marker.log".to_owned(),
                args: vec![],
            })
            .await;

        // And waiting for the spawned task to report completion.
        let _finish = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let finishes = audit.of_type::<FinishSessionTeardown>();
                if let Some(f) = finishes.into_iter().next() {
                    return f;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("FinishSessionTeardown must be emitted after teardown spawn");

        // Then the marker file landed inside the session's CWD (the tempdir),
        // proving the teardown shell process ran in the inherited session dir,
        // not jinn's process working directory.
        let marker = dir.path().join("teardown-marker.log");
        assert!(
            marker.exists(),
            "teardown marker file not found at {}",
            marker.display()
        );
    }
}
