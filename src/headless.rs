//! Headless application state.
//!
//! [`HeadlessApp`] owns the processing pipeline for non-interactive mode.
//! It receives commands, submits them to the core, and shuts down the actor
//! host gracefully.

use error_stack::{Report, ResultExt};
use nullslop_domain::ActorHostService;
use nullslop_domain::Command;
use nullslop_domain::EnqueueUserMessage;
use nullslop_domain::IntentHandler;
use nullslop_domain::{AppCore, AppMsg};
use wherror::Error;

/// Error type for headless operations.
#[derive(Debug, Error)]
#[error(debug)]
pub struct HeadlessError;

/// Headless application state.
///
/// Owns an [`AppCore`] for non-interactive command processing.
/// Commands are submitted and results can be inspected after shutdown.
pub struct HeadlessApp {
    /// Application core (state, message channel).
    core: AppCore,
    /// Actor host for coordinated shutdown.
    actor_host: ActorHostService,
    /// Receiver for core lifecycle notifications (shutdown complete).
    core_receiver: kanal::Receiver<nullslop_domain::CoreNotification>,
    /// Tokio runtime handle for spawning async shutdown task.
    handle: tokio::runtime::Handle,
}

impl HeadlessApp {
    /// Creates a new headless app with the given core, actor host, and core receiver.
    #[must_use]
    pub fn new(
        core: AppCore,
        actor_host: ActorHostService,
        core_receiver: kanal::Receiver<nullslop_domain::CoreNotification>,
        handle: tokio::runtime::Handle,
    ) -> Self {
        Self {
            core,
            actor_host,
            core_receiver,
            handle,
        }
    }

    /// Sends a chat message through the core pipeline.
    ///
    /// # Errors
    ///
    /// Returns an error if the message cannot be sent.
    pub fn send_chat(&self, message: &str) -> Result<(), Report<HeadlessError>> {
        let session_id = {
            let state = self.core.state.read();
            state.session.active_session.clone()
        };
        self.core
            .sender()
            .send(AppMsg::Command {
                command: Command::EnqueueUserMessage {
                    payload: EnqueueUserMessage {
                        session_id,
                        text: message.to_string(),
                    },
                },
                source: None,
            })
            .change_context(HeadlessError)
            .attach("failed to send chat command")
    }

    /// Runs a keystroke script through the keymap → command → bus → component pipeline.
    ///
    /// Each non-empty, non-comment line read from `reader` is parsed as a key
    /// sequence by [`parse_script`]. Keys are fed to the which-key state machine,
    /// which resolves them to commands. Commands are submitted to `AppCore`.
    ///
    /// # Errors
    ///
    /// Returns an error if the script content cannot be read or a command cannot
    /// be sent.
    pub fn run_script<R>(&mut self, mut reader: R) -> Result<(), Report<HeadlessError>>
    where
        R: std::io::Read,
    {
        let keymap = nullslop_tui::keymap::init();
        let mut which_key =
            nullslop_tui::app::WhichKeyInstance::new(keymap, nullslop_tui::Scope::Normal);
        let leader = nullslop_domain::KeyEvent {
            key: nullslop_domain::Key::Char('\\'),
            modifiers: nullslop_domain::Modifiers::none(),
        };

        let mut content = String::new();
        reader
            .read_to_string(&mut content)
            .change_context(HeadlessError)
            .attach("failed to read script content")?;

        let lines = parse_script(&content, &leader);

        for keys in lines {
            for key in keys {
                let state_read = self.core.state.read();
                let scope = nullslop_tui::app::scope_for_mode(
                    state_read.frontend.mode,
                    state_read.frontend.active_tab,
                    false,
                );
                drop(state_read);
                which_key.set_scope(scope);

                if let Some(intent) = which_key.handle_key(key) {
                    // Process the intent through the IntentHandler.
                    let mut state = self.core.state.write();
                    let result = IntentHandler::handle(&intent, &mut state);
                    drop(state);

                    // Send resulting commands to core.
                    for cmd in result.commands {
                        self.core
                            .sender()
                            .send(AppMsg::Command {
                                command: cmd,
                                source: None,
                            })
                            .change_context(HeadlessError)
                            .attach("failed to send script command")?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Prints the chat history to the log for visibility.
    pub fn print_history(&self) {
        let state = self.core.state.read();
        for entry in state.active_session().history() {
            tracing::info!("{entry:?}");
        }
    }

    /// Shuts down the actor host gracefully.
    pub fn shutdown(&mut self) {
        nullslop_domain::coordinated_shutdown(
            self.actor_host.backend(),
            &self.core.state,
            &self.core_receiver,
            &self.handle,
            nullslop_domain::SHUTDOWN_TIMEOUT,
        );
    }
}

/// Parses a script's content into a list of key sequences.
///
/// Each non-empty, non-comment line is parsed into a `Vec<KeyEvent>` via
/// [`ratatui_which_key::parse_key_sequence`]. Blank lines and lines starting
/// with `#` are skipped. Returns one `Vec<KeyEvent>` per non-skipped line.
pub fn parse_script(
    content: &str,
    leader: &nullslop_domain::KeyEvent,
) -> Vec<Vec<nullslop_domain::KeyEvent>> {
    content
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| ratatui_which_key::parse_key_sequence(line, leader))
        .collect()
}

#[cfg(test)]
mod tests {
    use nullslop_domain::{Key, KeyEvent, Modifiers};

    use super::*;

    fn leader() -> KeyEvent {
        KeyEvent {
            key: Key::Char('\\'),
            modifiers: Modifiers::none(),
        }
    }

    // --- parse_script unit tests ---

    #[rstest::rstest]
    fn parse_script_skips_comment_lines() {
        // Given a script with comment lines.
        let content = "# This is a comment\nq\n# Another comment";

        // When parsing.
        let lines = parse_script(content, &leader());

        // Then only the non-comment line produces a sequence.
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), 1);
        assert_eq!(lines[0][0].key, Key::Char('q'));
    }

    #[rstest::rstest]
    fn parse_script_skips_blank_lines() {
        // Given a script with blank and whitespace-only lines.
        let content = "\n   \nq\n\n";

        // When parsing.
        let lines = parse_script(content, &leader());

        // Then only the non-blank line produces a sequence.
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0][0].key, Key::Char('q'));
    }

    #[rstest::rstest]
    fn parse_script_parses_single_key() {
        // Given a script with a single key.
        let content = "q";

        // When parsing.
        let lines = parse_script(content, &leader());

        // Then one line with one key is produced.
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), 1);
        assert_eq!(lines[0][0].key, Key::Char('q'));
    }

    #[rstest::rstest]
    fn parse_script_parses_special_key() {
        // Given a script with a special key name.
        let content = "<enter>";

        // When parsing.
        let lines = parse_script(content, &leader());

        // Then one line with one Enter key is produced.
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), 1);
        assert_eq!(lines[0][0].key, Key::Enter);
    }

    #[rstest::rstest]
    fn parse_script_multi_key_produces_correct_counts() {
        // Given a script with a multi-key sequence.
        let content = "ihello<enter>";

        // When parsing.
        let lines = parse_script(content, &leader());

        // Then 1 line with 7 keys is produced.
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), 7);
    }

    #[rstest::rstest]
    #[case::i(0, Key::Char('i'))]
    #[case::h(1, Key::Char('h'))]
    #[case::e(2, Key::Char('e'))]
    #[case::l_1(3, Key::Char('l'))]
    #[case::l_2(4, Key::Char('l'))]
    #[case::o(5, Key::Char('o'))]
    #[case::enter(6, Key::Enter)]
    fn parse_script_multi_key_matches_expected_key(#[case] index: usize, #[case] expected: Key) {
        // Given a script with a multi-key sequence.
        let content = "ihello<enter>";

        // When parsing.
        let lines = parse_script(content, &leader());

        // Then the key at the given index matches.
        assert_eq!(lines[0][index].key, expected);
    }

    #[rstest::rstest]
    fn parse_script_handles_multiple_lines() {
        // Given a multi-line script.
        let content = "i\nhello<enter>\nq";

        // When parsing.
        let lines = parse_script(content, &leader());

        // Then three sequences are produced.
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].len(), 1); // i
        assert_eq!(lines[1].len(), 6); // h, e, l, l, o, enter
        assert_eq!(lines[2].len(), 1); // q
    }
}
