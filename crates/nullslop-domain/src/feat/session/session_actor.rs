//! Session lifecycle and persistence actor - owns session state from input to streaming.
//!
//! This actor is the **sole owner** of session-related state: chat history, input
//! buffers, session phase transitions, tool call state, and streaming tokens. It
//! also handles persisting sessions to disk and restoring them on load.
//!
//! # State ownership
//!
//! This actor is the **sole writer** of the following `AppState` fields:
//! - session history (entries, tool calls, streaming state)
//! - session input buffers
//! - session phase (idle → sending → streaming → idle)
//! - `active_session`, `session_load_guard`
//!
//! # Lock discipline
//!
//! All handlers follow the same pattern: acquire state lock → mutate → release →
//! then emit. Never hold the lock during emission.

mod handlers;

use super::SessionStoreService;

use crate::SessionForkRequested;
use crate::SessionLoadRequested;
use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::chat_input::protocol::command::{
    EnqueueUserMessage, PushChatEntry, SetChatInputText,
};
use crate::feat::context::protocol::command::SwitchPromptStrategy;
use crate::feat::context::protocol::event::PromptAssembled;
use crate::feat::context::strategy::token_estimator::TiktokenCounter;
use crate::feat::provider::protocol::command::SendMessage;
use crate::feat::provider::protocol::event::{ModelsRefreshed, StreamCompleted, StreamToken};
use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::session::protocol::load_session_picker_entries::LoadSessionPickerEntries;
use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted;
use crate::feat::session_lifecycle::command_runner::LifecycleCommandError;
use crate::feat::session_lifecycle::command_runner::run_setup_command;
use crate::feat::session_lifecycle::command_runner::run_teardown_command;
use crate::feat::session_lifecycle::protocol::command::{RunSessionSetup, RunSessionTeardown};
use crate::feat::session_lifecycle::protocol::event::{
    SessionSetupCompleted, SessionTeardownCompleted,
};
use crate::feat::tools_actor::protocol::event::{
    ToolCallReceived, ToolCallStreaming, ToolExecutionCompleted, ToolUseStarted,
};
use crate::init::EnvironmentLoaded;
use crate::protocol::{ChatEntry, Command, Event, PromptStrategyId};

/// Session lifecycle and persistence actor.
///
/// Subscribes to session-related commands and events, mutates [`State`],
/// and emits new commands and events via the [`ActorContext`] message sink.
/// Also persists session snapshots to disk when session state changes.
pub struct SessionPersistenceActor {
    /// Shared application state.
    pub(super) state: State,
    /// Runtime services (user preferences storage for startup config loading).
    pub(super) services: Option<Services>,
    /// The session store service for writing session snapshots.
    pub(super) store: Option<SessionStoreService>,
    /// Token counter for recording token usage in the session ledger.
    pub(super) counter: TiktokenCounter,
}

/// Remove ANSI escape sequences (CSI SGR codes) from a string.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            // Consume '['.
            if chars.next() != Some('[') {
                out.push(ch);
                continue;
            }
            // Consume parameter bytes (digits, semicolons, '?').
            while let Some(&next) = chars.as_str().as_bytes().first() {
                if next.is_ascii_digit() || next == b';' || next == b'?' {
                    chars.next();
                } else {
                    break;
                }
            }
            // Consume the final byte (m, K, H, etc.).
            chars.next();
        } else {
            out.push(ch);
        }
    }
    out
}

/// Build an info chat entry for when a setup command produces no output.
///
/// Shows a yellow "No path returned" line and a white line with the fallback CWD.
fn no_output_info(default_cwd: &std::path::Path) -> ChatEntry {
    use ratatui::style::{Color, Style};
    use ratatui::text::{Line, Span};

    ChatEntry::info(vec![
        Line::from(Span::styled(
            "No path returned by setup command.",
            Style::default().fg(Color::Yellow),
        )),
        Line::from(Span::styled(
            format!("Using {} as cwd", default_cwd.display()),
            Style::default().fg(Color::White),
        )),
    ])
}

/// INFO entry shown while a setup command is running.
fn setup_running_msg() -> ChatEntry {
    use ratatui::style::{Color, Style};
    use ratatui::text::{Line, Span};

    ChatEntry::info(vec![Line::from(Span::styled(
        "⚙️ Running setup script...",
        Style::default().fg(Color::Yellow),
    ))])
}

/// INFO entry shown when a setup command completes successfully.
fn setup_complete_msg(cwd: &std::path::Path) -> ChatEntry {
    use ratatui::style::{Color, Style};
    use ratatui::text::{Line, Span};

    ChatEntry::info(vec![Line::from(Span::styled(
        format!("✅ Setup complete — Using {} as cwd", cwd.display()),
        Style::default().fg(Color::Green),
    ))])
}

/// INFO entry shown while a teardown command is running.
fn teardown_running_msg() -> ChatEntry {
    use ratatui::style::{Color, Style};
    use ratatui::text::{Line, Span};

    ChatEntry::info(vec![Line::from(Span::styled(
        "⚙️ Running teardown script...",
        Style::default().fg(Color::Yellow),
    ))])
}

/// Format a `LifecycleCommandError` into a clean user-facing message.
fn format_lifecycle_error(err: &LifecycleCommandError) -> String {
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

impl Actor for SessionPersistenceActor {
    type Message = NoDirectMsg;

    fn activate(ctx: &mut ActorContext) -> Self {
        // Persistence subscriptions.
        ctx.subscribe_command::<SessionLoadRequested>();
        ctx.subscribe_command::<LoadSessionPickerEntries>();
        ctx.subscribe_command::<SessionForkRequested>();

        // Session lifecycle subscriptions.
        ctx.subscribe_command::<EnqueueUserMessage>();
        ctx.subscribe_command::<SetChatInputText>();
        ctx.subscribe_command::<PushChatEntry>();
        ctx.subscribe_command::<SendMessage>();
        ctx.subscribe_command::<SessionLoadCompleted>();

        // Lifecycle command subscriptions.
        ctx.subscribe_command::<RunSessionSetup>();
        ctx.subscribe_command::<RunSessionTeardown>();

        // Event subscriptions.
        ctx.subscribe_event::<PromptAssembled>();
        ctx.subscribe_event::<StreamToken>();
        ctx.subscribe_event::<StreamCompleted>();
        ctx.subscribe_event::<ToolUseStarted>();
        ctx.subscribe_event::<ToolCallReceived>();
        ctx.subscribe_event::<ToolCallStreaming>();
        ctx.subscribe_event::<ToolExecutionCompleted>();
        ctx.subscribe_event::<crate::feat::context::protocol::event::ChatEntryPinChanged>();
        ctx.subscribe_event::<ModelsRefreshed>();
        ctx.subscribe_event::<EnvironmentLoaded>();

        ctx.set_description("Session lifecycle and persistence");

        #[expect(clippy::expect_used, reason = "State is always injected at startup")]
        let state = ctx
            .take_data::<State>()
            .expect("SessionPersistenceActor requires State injection");
        let store = ctx.take_data::<SessionStoreService>();
        let services = ctx.take_data::<Services>();
        let counter = ctx
            .take_data::<TiktokenCounter>()
            .unwrap_or_else(TiktokenCounter::o200k_base);

        Self {
            state,
            services,
            store,
            counter,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(event) => self.handle_event(&event, ctx).await,
            ActorEnvelope::Command(cmd) => self.handle_command(&cmd, ctx).await,
            _ => {}
        }
    }

    async fn on_shutdown(&mut self, _ctx: &ActorContext) {
        self.run_pending_teardowns().await;
    }
}

impl SessionPersistenceActor {
    /// Dispatches a bus event to the appropriate handler.
    async fn handle_event(&mut self, event: &Event, ctx: &ActorContext) {
        match event {
            Event::PromptAssembled(payload) => self.handle_prompt_assembled(payload, ctx),
            Event::StreamToken(payload) => self.on_stream_token(payload),
            Event::StreamCompleted(payload) => self.on_stream_completed(payload, ctx).await,
            Event::ToolUseStarted(payload) => self.on_tool_use_started(payload),
            Event::ToolCallReceived(payload) => self.on_tool_call_received(payload),
            Event::ToolCallStreaming(payload) => self.on_tool_call_streaming(payload),
            Event::ToolExecutionCompleted(payload) => {
                self.on_tool_execution_completed(payload).await;
            }
            Event::ModelsRefreshed(payload) => {
                self.on_models_refreshed(payload);
            }
            Event::EnvironmentLoaded(payload) => {
                self.on_environment_loaded(&payload.config, ctx);
            }
            Event::ChatEntryPinChanged(payload) => {
                self.save_active_session(&payload.session_id).await;
            }
            _ => {}
        }
    }

    /// Dispatches a command to the appropriate handler.
    async fn handle_command(&mut self, cmd: &Command, ctx: &ActorContext) {
        match cmd {
            Command::SessionLoadRequested(payload) => self.on_load_requested(payload, ctx).await,
            Command::SessionForkRequested(payload) => {
                self.on_session_fork_requested(payload, ctx).await;
            }
            Command::LoadSessionPickerEntries(payload) => {
                self.handle_load_session_picker_entries(payload).await;
            }
            Command::EnqueueUserMessage(payload) => {
                self.handle_enqueue_user_message(payload, ctx).await;
            }
            Command::SetChatInputText(payload) => self.handle_set_chat_input_text(payload),
            Command::PushChatEntry(payload) => self.handle_push_chat_entry(payload, ctx),
            Command::SendMessage(payload) => Self::handle_send_message(payload, ctx),
            Command::SessionLoadCompleted(payload) => {
                self.handle_session_load_completed(payload, ctx).await;
            }
            Command::RunSessionSetup(payload) => {
                self.handle_run_session_setup(payload, ctx).await;
            }
            Command::RunSessionTeardown(payload) => {
                self.handle_run_session_teardown(payload, ctx).await;
            }
            // Commands NOT subscribed to - these should not arrive.
            Command::AssemblePrompt(..)
            | Command::SendToLlmProvider(..)
            | Command::ExecuteTool(..)
            | Command::ProceedWithShutdown(..)
            | Command::CancelStream(..)
            | Command::RefreshModels
            | Command::RescanPromptTemplates
            | Command::ExecuteToolBatch(..)
            | Command::RegisterTools(..)
            | Command::ProviderSwitch(..)
            | Command::LoadProviderPickerEntries(..)
            | Command::LoadContextStrategyPickerEntries(..)
            | Command::PinChatEntry(..)
            | Command::UnpinChatEntry(..)
            | Command::SwitchPromptStrategy(..)
            | Command::RestoreStrategyState(..)
            | Command::CancelToolBatch(..)
            | Command::ScanSkills
            | Command::RescanPersonas(..)
            | Command::LoadPersonaPickerEntries(..)
            | Command::UpdatePreferences(..) => {}
        }
    }

    /// Loads session picker entries from the session store into `AppState`.
    async fn handle_load_session_picker_entries(&self, _payload: &LoadSessionPickerEntries) {
        if let Some(ref store) = self.store {
            let theme = {
                let state = self.state.read();
                state.frontend.theme.clone()
            };
            let entries =
                crate::feat::session::entries::load_session_entries_from_store(store, &theme).await;
            let mut state = self.state.write();
            state.frontend.session_picker.set_items(entries);
        }
    }

    /// Applies config defaults to the default session profile on startup.
    ///
    /// Loads user preferences and applies `last_model` and `last_strategy`
    /// to the default session, then sends an `UpdatePreferences` command so
    /// the preferences pipeline handles persistence and state sync.
    ///
    /// NOTE: Using `active_session_mut()` is acceptable here because this runs
    /// at startup before any user interaction. There is only one session.
    fn on_environment_loaded(
        &self,
        _config: &crate::feat::provider_infra::ProvidersConfig,
        ctx: &ActorContext,
    ) {
        let Some(ref services) = self.services else {
            return;
        };

        let prefs = match services.user_preferences_storage.load() {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(err = ?e, "session-actor failed to load preferences on startup");
                return;
            }
        };

        let session_id;
        {
            let mut state = self.state.write();

            // Apply config defaults to the default session.
            let session = state.active_session_mut();
            if let Some(ref model) = prefs.last_model {
                session.set_model(model.clone());
            }
            if let Some(ref strategy_str) = prefs.last_strategy {
                let strategy_id = PromptStrategyId::new(strategy_str.clone());
                session.switch_strategy(strategy_id.clone());
            }
            session_id = state.session.active_session.clone();
        }

        // Send UpdatePreferences command so the pipeline handles persistence + state sync.
        if let Err(e) = ctx.send_command(Command::UpdatePreferences(crate::feat::preferences_actor::protocol::command::UpdatePreferences {
                updates: vec![
                    crate::feat::preferences_actor::protocol::command::PreferenceUpdate::SetLastModel(prefs.last_model.clone()),
                    crate::feat::preferences_actor::protocol::command::PreferenceUpdate::SetLastStrategy(prefs.last_strategy.clone()),
                ],
            })) {
            tracing::warn!(err = ?e, "session-actor failed to send UpdatePreferences on startup");
        }

        // Emit SwitchPromptStrategy so the context actor initializes the strategy.
        if let Some(ref strategy_str) = prefs.last_strategy {
            let strategy_id = PromptStrategyId::new(strategy_str.clone());
            if let Err(e) = ctx.send_command(Command::SwitchPromptStrategy(SwitchPromptStrategy {
                session_id,
                strategy_id,
            })) {}
        }
    }

    /// RunSessionSetup: execute the lifecycle setup command asynchronously.
    ///
    /// On success, sets the session's CWD to the command's output.
    /// On failure, sets the default CWD and pushes an error entry.
    async fn handle_run_session_setup(&self, payload: &RunSessionSetup, ctx: &ActorContext) {
        // Push "running" info entry so the user sees feedback immediately.
        {
            let mut state = self.state.write();
            if let Some(session) = state.session.sessions.get_mut(&payload.session_id) {
                session.push_entry(setup_running_msg());
            }
        }

        let result = run_setup_command(&payload.command).await;

        match result {
            Ok(cwd) => {
                let mut state = self.state.write();
                if let Some(session) = state.session.sessions.get_mut(&payload.session_id) {
                    session.set_cwd(cwd.clone());
                    session.push_entry(setup_complete_msg(&cwd));
                }
                drop(state);

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
                    let default = state.session.default_cwd.clone();
                    if let Some(session) = state.session.sessions.get_mut(&payload.session_id) {
                        session.set_cwd(default.clone());
                        let entry = if is_no_output {
                            no_output_info(&default)
                        } else {
                            ChatEntry::error(&error_msg)
                        };
                        session.push_entry(entry);
                    }
                    default
                };

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

    /// RunSessionTeardown: execute the lifecycle teardown command asynchronously.
    ///
    /// On success, removes the session from the map and switches to another.
    /// On failure, pushes an error entry and keeps the session open.
    async fn handle_run_session_teardown(&self, payload: &RunSessionTeardown, ctx: &ActorContext) {
        // Push "running" info entry so the user sees feedback immediately.
        {
            let mut state = self.state.write();
            if let Some(session) = state.session.sessions.get_mut(&payload.session_id) {
                session.push_entry(teardown_running_msg());
            }
        }

        let result = run_teardown_command(&payload.command).await;

        match result {
            Ok(()) => {
                // Teardown succeeded - remove session and switch active.
                {
                    let mut state = self.state.write();
                    state.session.sessions.remove(&payload.session_id);
                    if state.session.sessions.is_empty() {
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
                        let new_session = ChatSessionState::new_with_profile(
                            crate::feat::session::profile::SessionProfile::from_config(
                                model, strategy,
                            ),
                        );
                        let new_id = new_session.session_id().clone();
                        state.session.sessions.insert(new_id.clone(), new_session);
                        state.session.active_session = new_id;
                    } else if state.session.active_session == payload.session_id {
                        let next_id = state
                            .session
                            .sessions
                            .keys()
                            .next()
                            .expect("sessions is non-empty")
                            .clone();
                        state.session.active_session = next_id;
                    }
                }

                if let Err(e) =
                    ctx.send_event(Event::SessionTeardownCompleted(SessionTeardownCompleted {
                        session_id: payload.session_id.clone(),
                        error: None,
                    }))
                {
                    tracing::warn!(err = ?e, "session-actor failed to emit SessionTeardownCompleted");
                }
            }
            Err(report) => {
                let error_msg =
                    if let Some(cmd_err) = report.downcast_ref::<LifecycleCommandError>() {
                        format_lifecycle_error(cmd_err)
                    } else {
                        strip_ansi(&format!("{report:#?}"))
                    };
                {
                    let mut state = self.state.write();
                    state.session.active_session = payload.session_id.clone();
                    if let Some(session) = state.session.sessions.get_mut(&payload.session_id) {
                        session.push_entry(ChatEntry::error(&error_msg));
                    }
                    state
                        .frontend
                        .scope_stack
                        .push(crate::common::app_state::FocusScope::Input);
                }

                if let Err(e) =
                    ctx.send_event(Event::SessionTeardownCompleted(SessionTeardownCompleted {
                        session_id: payload.session_id.clone(),
                        error: Some(error_msg),
                    }))
                {
                    tracing::warn!(err = ?e, "session-actor failed to emit SessionTeardownCompleted");
                }
            }
        }
    }

    /// Runs teardown commands for all open sessions that have a lifecycle with teardown.
    ///
    /// Called during coordinated shutdown. Runs commands sequentially -
    /// teardown order matters (each must complete before the next starts).
    async fn run_pending_teardowns(&self) {
        use crate::feat::session_lifecycle::command_template::CommandTemplate;

        let teardown_jobs: Vec<(crate::protocol::SessionId, String, String)> = {
            let state = self.state.read();
            let mut jobs = Vec::new();
            for (id, session) in &state.session.sessions {
                let Some(lifecycle_name) = session.lifecycle_name() else {
                    continue;
                };
                let teardown_cmd = state
                    .frontend
                    .preferences
                    .session_lifecycles
                    .iter()
                    .find(|l| l.name == lifecycle_name)
                    .and_then(|l| l.teardown_command.clone());
                let Some(teardown_cmd) = teardown_cmd else {
                    continue;
                };
                let args = session.lifecycle_args().to_vec();
                let template = CommandTemplate::parse(&teardown_cmd);
                let rendered = if args.is_empty() {
                    teardown_cmd
                } else {
                    template.render(&args)
                };
                jobs.push((id.clone(), lifecycle_name.to_owned(), rendered));
            }
            jobs
        };

        for (session_id, lifecycle_name, command) in teardown_jobs {
            tracing::info!(
                session_id = %session_id,
                lifecycle = %lifecycle_name,
                "running teardown during shutdown"
            );
            match run_teardown_command(&command).await {
                Ok(()) => {
                    tracing::info!(
                        session_id = %session_id,
                        "teardown completed during shutdown"
                    );
                }
                Err(report) => {
                    tracing::warn!(
                        session_id = %session_id,
                        err = ?report,
                        "teardown failed during shutdown"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        no_output_info, setup_complete_msg, setup_running_msg, strip_ansi, teardown_running_msg,
    };
    use crate::protocol::ChatEntryKind;
    use ratatui::style::Color;
    use std::path::Path;

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
    fn no_output_info_is_info_entry_with_two_lines() {
        // Given a default CWD path.
        let cwd = Path::new("/tmp/test-project");

        // When building the no-output info entry.
        let entry = no_output_info(cwd);

        // Then it is an Info entry with exactly 2 lines.
        let ChatEntryKind::Info(lines) = &entry.kind else {
            panic!("expected Info entry, got {:?}", entry.kind);
        };
        assert_eq!(lines.len(), 2);
    }

    #[rstest::rstest]
    fn no_output_info_first_line_is_yellow() {
        // Given a default CWD path.
        let cwd = Path::new("/tmp/test-project");

        // When building the no-output info entry.
        let entry = no_output_info(cwd);

        // Then the first line has yellow foreground.
        let ChatEntryKind::Info(lines) = &entry.kind else {
            panic!("expected Info entry");
        };
        let first_span = &lines[0].spans[0];
        assert_eq!(first_span.content, "No path returned by setup command.");
        assert_eq!(first_span.style.fg, Some(Color::Yellow));
    }

    #[rstest::rstest]
    fn no_output_info_second_line_is_white_with_cwd() {
        // Given an absolute default CWD path.
        let cwd = Path::new("/tmp/test-project");

        // When building the no-output info entry.
        let entry = no_output_info(cwd);

        // Then the second line is white and contains the absolute CWD.
        let ChatEntryKind::Info(lines) = &entry.kind else {
            panic!("expected Info entry");
        };
        let second_span = &lines[1].spans[0];
        assert_eq!(second_span.style.fg, Some(Color::White));
        assert!(second_span.content.contains("/tmp/test-project"));
        assert!(second_span.content.starts_with("Using "));
        assert!(second_span.content.ends_with(" as cwd"));
    }

    #[rstest::rstest]
    fn setup_running_msg_is_yellow_with_gear_emoji() {
        // When building the setup running message.
        let entry = setup_running_msg();

        // Then it is an Info entry with one line.
        let ChatEntryKind::Info(lines) = &entry.kind else {
            panic!("expected Info entry");
        };
        assert_eq!(lines.len(), 1);
        let span = &lines[0].spans[0];
        assert!(span.content.contains("⚙️"));
        assert!(span.content.contains("Running setup script"));
        assert_eq!(span.style.fg, Some(Color::Yellow));
    }

    #[rstest::rstest]
    fn setup_complete_msg_is_green_with_checkmark_and_cwd() {
        // Given a CWD path.
        let cwd = Path::new("/tmp/my-project");

        // When building the setup complete message.
        let entry = setup_complete_msg(cwd);

        // Then it is an Info entry with the checkmark and CWD.
        let ChatEntryKind::Info(lines) = &entry.kind else {
            panic!("expected Info entry");
        };
        assert_eq!(lines.len(), 1);
        let span = &lines[0].spans[0];
        assert!(span.content.contains("✅"));
        assert!(span.content.contains("Setup complete"));
        assert!(span.content.contains("Using /tmp/my-project as cwd"));
        assert_eq!(span.style.fg, Some(Color::Green));
    }

    #[rstest::rstest]
    fn teardown_running_msg_is_yellow_with_gear_emoji() {
        // When building the teardown running message.
        let entry = teardown_running_msg();

        // Then it is an Info entry with one line.
        let ChatEntryKind::Info(lines) = &entry.kind else {
            panic!("expected Info entry");
        };
        assert_eq!(lines.len(), 1);
        let span = &lines[0].spans[0];
        assert!(span.content.contains("⚙️"));
        assert!(span.content.contains("Running teardown script"));
        assert_eq!(span.style.fg, Some(Color::Yellow));
    }
}
