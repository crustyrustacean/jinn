//! Welcome subscriber — translates dynamic commands into typed commands.
//!
//! Listens for the `"welcome::show"` dynamic command and translates it
//! into a typed `PushChatEntry` command so the message appears in the
//! chat UI.

use nullslop_domain::DynamicCommand;
use nullslop_domain::ChatEntry;
use nullslop_domain::SessionId;
use nullslop_domain::feat::chat_input::protocol::command::PushChatEntry;

use crate::host::CommandSender;

/// Translates `"welcome::show"` dynamic commands into typed `PushChatEntry`.
pub struct WelcomeSubscriber {
    /// The command sender callback.
    sender: CommandSender,
}

impl WelcomeSubscriber {
    /// Creates a new welcome subscriber with the given command sender.
    pub fn new(sender: CommandSender) -> Self {
        Self { sender }
    }

    /// Handles a dynamic command.
    ///
    /// If the command is `"welcome::show"`, extracts the message from the
    /// payload (defaulting to a generic welcome) and emits a typed
    /// `PushChatEntry` with a system chat entry.
    pub fn handle(&self, cmd: &DynamicCommand, session_id: &SessionId) {
        if cmd.name != "welcome::show" {
            return;
        }

        let message = cmd
            .payload
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Welcome to nullslop! Press ? for keybindings.");

        let entry = ChatEntry::system(message);
        let push = PushChatEntry {
            session_id: session_id.clone(),
            entry,
        };

        self.sender.send(nullslop_domain::Command::PushChatEntry(push));
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        reason = "test code, panics are acceptable"
    )]
    use nullslop_domain::Command;

    use super::*;

    fn test_setup() -> (WelcomeSubscriber, kanal::Receiver<Command>) {
        let (tx, rx) = kanal::unbounded();
        let sender = CommandSender::new(move |cmd: Command| {
            let _ = tx.send(cmd);
        });
        (WelcomeSubscriber::new(sender), rx)
    }

    #[rstest::rstest]
    fn welcome_show_sends_push_chat_entry() {
        // Given a welcome subscriber.
        let (sub, rx) = test_setup();
        let session_id = SessionId::new();

        // When handling a welcome::show command.
        let cmd = DynamicCommand {
            name: "welcome::show".to_owned(),
            payload: serde_json::json!({ "message": "Hello, plugin!" }),
        };
        sub.handle(&cmd, &session_id);

        // Then a PushChatEntry command is emitted.
        let result = rx.recv().expect("should receive command");
        match result {
            Command::PushChatEntry(pce) => {
                assert_eq!(pce.session_id, session_id);
                // The system message should contain our custom text.
                let text = pce.entry.text();
                assert!(text.contains("Hello, plugin!"), "message should contain custom text");
            }
            other => panic!("expected PushChatEntry, got {other:?}"),
        }
    }

    #[rstest::rstest]
    fn welcome_show_uses_default_message_when_missing() {
        // Given a welcome subscriber.
        let (sub, rx) = test_setup();
        let session_id = SessionId::new();

        // When handling a welcome::show with no message in payload.
        let cmd = DynamicCommand {
            name: "welcome::show".to_owned(),
            payload: serde_json::Value::Null,
        };
        sub.handle(&cmd, &session_id);

        // Then the default welcome message is used.
        let result = rx.recv().expect("should receive command");
        match result {
            Command::PushChatEntry(pce) => {
                let text = pce.entry.text();
                assert!(
                    text.contains("Welcome to nullslop"),
                    "should use default message, got: {text}"
                );
            }
            other => panic!("expected PushChatEntry, got {other:?}"),
        }
    }

    #[rstest::rstest]
    fn non_welcome_command_is_ignored() {
        // Given a welcome subscriber.
        let (sub, rx) = test_setup();
        let session_id = SessionId::new();

        // When handling a different dynamic command.
        let cmd = DynamicCommand {
            name: "other::command".to_owned(),
            payload: serde_json::Value::Null,
        };
        sub.handle(&cmd, &session_id);

        // Then no command is emitted.
        let result = rx.try_recv().expect("try_recv should succeed");
        assert!(
            result.is_none(),
            "non-welcome commands should be ignored"
        );
    }
}
