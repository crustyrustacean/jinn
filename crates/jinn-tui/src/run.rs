//! Terminal setup, event loop, and teardown.
//!
//! Sets up the terminal (raw mode + alternate screen), runs the
//! main event loop, and restores the terminal on exit. Also manages
//! the background event thread lifecycle, stopping it before terminal
//! suspension and restarting it afterward.

use std::io::{self, Stdout};
use std::time::{Duration, Instant};

use crossterm::{
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use error_stack::{Report, ResultExt as _};
use ratatui::{Terminal, backend::CrosstermBackend};
use wherror::Error;

use crate::TuiApp;
use crate::app::scope_for_focus;
use crate::msg::Msg;

/// Error type for TUI run operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error(debug)]
pub struct TuiRunError;

/// Runs the TUI application.
///
/// Sets up the terminal, runs the main event loop, and restores
/// the terminal on exit. The caller must provide a fully-initialized
/// [`TuiApp`] with services already set.
///
/// # Errors
///
/// Returns an error if terminal setup, the event loop, or teardown fails.
pub fn run(mut app: TuiApp) -> Result<(), Report<TuiRunError>> {
    let mut stdout = io::stdout();
    enable_raw_mode()
        .change_context(TuiRunError)
        .attach("failed to enable raw mode")?;
    execute!(stdout, EnterAlternateScreen)
        .change_context(TuiRunError)
        .attach("failed to enter alternate screen")?;

    execute!(stdout, EnableBracketedPaste)
        .change_context(TuiRunError)
        .attach("failed to enable bracketed paste")?;

    let mouse_selection = app.config.mouse_selection;

    // Enable mouse capture so scroll wheel and click events are reported.
    if mouse_selection {
        execute!(stdout, EnableMouseCapture)
            .change_context(TuiRunError)
            .attach("failed to enable mouse capture")?;
    }

    // Enable Kitty keyboard protocol so crossterm can distinguish
    // modified special keys (e.g. Shift+Enter, Ctrl+Enter).
    // Terminals that don't support it silently ignore the sequence.
    execute!(
        stdout,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    )
    .change_context(TuiRunError)
    .attach("failed to push keyboard enhancement flags")?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)
        .change_context(TuiRunError)
        .attach("failed to create terminal")?;

    // Start the event poll thread (independent of tokio runtime).
    app.event_thread = Some(app.events.start_event_thread());

    let result = run_main_loop(&mut terminal, &mut app);

    // Clean up event thread.
    if let Some(mut guard) = app.event_thread.take() {
        guard.stop();
    }

    // Coordinated actor shutdown: signal the root supervisor, then race the
    // shutdown barrier against a 20-second timeout. Kameo cascades the stop
    // signal to every supervised child (spawn.rs:216-227), running their
    // `on_stop` hooks to flush buffers and finalize writes.
    {
        let root = app.services.root_supervisor.clone();
        let result = app.services.handle.block_on(async {
            let _ = root.stop_gracefully().await;
            tokio::time::timeout(Duration::from_secs(20), root.wait_for_shutdown()).await
        });
        if result.is_err() {
            tracing::warn!("actor shutdown timed out after 20s; proceeding");
        }
    }

    // Restore terminal.
    if let Err(e) = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags) {
        tracing::error!(err = ?e, "failed to pop keyboard enhancement flags");
    }
    if let Err(e) = execute!(terminal.backend_mut(), DisableBracketedPaste) {
        tracing::error!(err = ?e, "failed to disable bracketed paste");
    }
    if mouse_selection && let Err(e) = execute!(terminal.backend_mut(), DisableMouseCapture) {
        tracing::error!(err = ?e, "failed to disable mouse capture");
    }
    if let Err(e) = disable_raw_mode() {
        tracing::error!(err = ?e, "failed to disable raw mode");
    }
    if let Err(e) = execute!(terminal.backend_mut(), LeaveAlternateScreen) {
        tracing::error!(err = ?e, "failed to leave alternate screen");
    }
    if let Err(e) = terminal.show_cursor() {
        tracing::error!(err = ?e, "failed to show cursor");
    }

    result
}

/// Runs the main TUI event loop - receives events, processes state, and renders frames.
fn run_main_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut TuiApp,
) -> Result<(), Report<TuiRunError>> {
    let mut last_render: Option<Instant> = None;

    loop {
        // ── Phase 1: Wait for next message ──────────────────────────────
        let event = app
            .events
            .recv()
            .change_context(TuiRunError)
            .attach("event channel closed")?;

        let is_input = matches!(event, Msg::Input(crossterm::event::Event::Key(_)));

        // ── Phase 2: Handle event batch ─────────────────────────────────
        app.handle_msg(event);
        while let Some(event) = app.events.try_recv() {
            app.handle_msg(event);
        }

        // ── Phase 3: State read for quit/scope ──────────────────────────
        let state_read = app.core.state.read();
        let should_quit = state_read.frontend.should_quit;
        let scope = scope_for_focus(state_read.frontend.scope_stack.current());
        drop(state_read);
        app.which_key.set_scope(scope);

        // Check for pending suspend after event batch processing.
        if let Some(action) = app.suspend.take_action() {
            handle_suspend_action(terminal, app, action)?;
        }

        // ── Phase 4: Render ─────────────────────────────────────────────
        let should_render =
            is_input || last_render.is_none_or(|t| t.elapsed() >= Duration::from_millis(33));

        if should_render {
            terminal
                .draw(|frame| {
                    app.render(frame);
                })
                .change_context(TuiRunError)
                .attach("failed to draw frame")?;
            last_render = Some(Instant::now());
        }

        if should_quit {
            break;
        }
    }

    Ok(())
}

/// Result of a suspend/restore cycle.
enum SuspendResult {
    /// The edited content, to be written to the input buffer.
    EditContent(Option<String>),
    /// The selected directory path, to be set as session CWD.
    ChangeCwd(Option<std::path::PathBuf>),
}

/// Executes a suspend/restore cycle for the given action.
///
/// 1. Stops the background event thread
/// 2. Drains stale messages from the channel
/// 3. Suspends the terminal via [`TerminalGuard`](crate::terminal::TerminalGuard)
/// 4. Runs the external editor via `dialoguer::Editor`
/// 5. Invokes the `on_result` closure to produce the new input buffer content
/// 6. Restarts the event thread
/// 7. Redraws the terminal
/// 8. Writes the result directly to the active session's input box via `replace_all`
#[expect(
    clippy::too_many_lines,
    reason = "large match on SuspendAction variants"
)]
fn handle_suspend_action(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut TuiApp,
    action: crate::suspend::SuspendAction,
) -> Result<(), Report<TuiRunError>> {
    // Stop the event thread so crossterm stops polling the terminal.
    if let Some(mut guard) = app.event_thread.take() {
        guard.stop();
    }
    app.events.drain();

    let result = crate::terminal::suspend_and_run(terminal, || match action {
        crate::suspend::SuspendAction::Edit {
            initial_content,
            on_result,
        } => {
            let edited = dialoguer::Editor::new()
                .edit(&initial_content)
                .ok()
                .flatten();

            let changed = edited.filter(|c| c != &initial_content);
            SuspendResult::EditContent(on_result(changed))
        }
        crate::suspend::SuspendAction::ChangeCwd { search_root } => {
            let command_template = app
                .core
                .state
                .read()
                .frontend
                .preferences
                .cwd_selector
                .command
                .clone();

            // Shell-escape the path for safe substitution.
            let escaped_path = shell_escape(&search_root.to_string_lossy());
            let rendered = command_template.replace("{path}", &escaped_path);

            let output_result = std::process::Command::new("sh")
                .arg("-c")
                .arg(&rendered)
                .current_dir(&search_root)
                .output();

            let selected = match output_result {
                Ok(output) => {
                    if output.status.success() {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        stdout
                            .lines()
                            .next()
                            .filter(|line| !line.is_empty())
                            .map(std::path::PathBuf::from)
                    } else {
                        // Non-zero exit = user cancelled (ESC in fzf) or error.
                        // Log stderr as debug - user cancelled is not an error.
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        if !stderr.is_empty() {
                            tracing::debug!(
                                stderr = %stderr,
                                exit_code = output.status.code().unwrap_or(-1),
                                "CWD selector exited with error"
                            );
                        }
                        None
                    }
                }
                Err(e) => {
                    tracing::error!(err = %e, "failed to run CWD selector command");
                    None
                }
            };
            SuspendResult::ChangeCwd(selected)
        }
    })
    .change_context(TuiRunError)
    .attach("failed to suspend terminal for editor")?;

    // Restart the event poll thread with a fresh crossterm state.
    app.event_thread = Some(app.events.start_event_thread());

    terminal
        .draw(|frame| {
            app.render(frame);
        })
        .change_context(TuiRunError)
        .attach("failed to redraw after suspend")?;

    // Handle the suspend result.
    match result {
        SuspendResult::EditContent(content) => {
            if let Some(content) = content {
                app.core
                    .state
                    .write()
                    .active_chat_input_mut()
                    .replace_all(content);
            }
        }
        SuspendResult::ChangeCwd(path) => {
            if let Some(path) = path {
                // Validate: must exist and be a directory.
                if path.is_dir() {
                    let canonical = match std::fs::canonicalize(&path) {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::warn!(
                                path = %path.display(),
                                err = %e,
                                "failed to canonicalize CWD selector result"
                            );
                            return Ok(());
                        }
                    };
                    let session_id = app.core.state.read().active_session().session_id().clone();
                    app.core.sender().send(jinn_domain::Bridge::publish_closure(
                        jinn_domain::feat::session_lifecycle::protocol::command::SetSessionCwd {
                            session_id,
                            cwd: canonical,
                        },
                    )).ok();

                    tracing::info!(
                        cwd = %app.core.state.read().active_session().cwd().display(),
                        "session CWD updated"
                    );
                } else {
                    tracing::warn!(
                        path = %path.display(),
                        "CWD selector returned non-directory path, ignoring"
                    );
                }
            }
        }
    }

    Ok(())
}

/// Shell-escapes a path for safe substitution into a `sh -c` command.
///
/// Wraps in single quotes, escaping any embedded single quotes.
fn shell_escape(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 2);
    result.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            result.push_str("'\\''");
        } else {
            result.push(ch);
        }
    }
    result.push('\'');
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn shell_escape_simple_path() {
        assert_eq!(shell_escape("/home/user/project"), "'/home/user/project'");
    }

    #[rstest::rstest]
    fn shell_escape_path_with_spaces() {
        assert_eq!(
            shell_escape("/home/user/my project"),
            "'/home/user/my project'"
        );
    }

    #[rstest::rstest]
    fn shell_escape_path_with_single_quote() {
        assert_eq!(
            shell_escape("/home/user/it's/project"),
            "'/home/user/it'\\''s/project'"
        );
    }

    #[rstest::rstest]
    fn shell_escape_empty_string() {
        assert_eq!(shell_escape(""), "''");
    }
}
