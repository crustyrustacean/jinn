//! Plugin wiring — concrete translator for plugin command names → typed Commands.
//!
//! This is the **only** file that knows about both plugin command names
//! (strings) and domain command types (Rust structs). The plugin crate
//! calls the translator blindly; this file provides the mapping.
//!
//! To add a new plugin command:
//! 1. Add a match arm in [`build_translator`]
//! 2. Update the plugin's Lua script to use the new command name

use nullslop_domain::ChatEntry;
use nullslop_domain::Command;
use nullslop_domain::SessionId;
use nullslop_domain::feat::chat_input::protocol::command::PushChatEntry;
use nullslop_plugin::TranslatorFn;

/// Builds the concrete translator that maps plugin command names to typed Commands.
///
/// The translator is a closure that receives a command name string and a JSON
/// payload, and returns `Some(Command)` if the name is recognized.
///
/// # Supported commands
///
/// | Name | Maps to |
/// |------|---------|
/// | `"push_chat_entry"` | `Command::PushChatEntry` with system entry |
/// | `"push_chat_entry_transient"` | `Command::PushChatEntry` with transient entry |
#[must_use]
pub fn build_translator() -> TranslatorFn {
    std::sync::Arc::new(|name: &str, payload: serde_json::Value| match name {
        "push_chat_entry" => translate_push_chat_entry(&payload, ChatEntry::system),
        "push_chat_entry_transient" => translate_push_chat_entry(&payload, ChatEntry::transient),
        _ => {
            tracing::warn!(name, "unknown plugin command name");
            None
        }
    })
}

/// Translates a `push_chat_entry` payload into a typed `Command::PushChatEntry`.
///
/// Expects the payload to have:
/// - `session_id`: string (required)
/// - `message`: string (optional, defaults to generic message)
fn translate_push_chat_entry(
    payload: &serde_json::Value,
    make_entry: fn(String) -> ChatEntry,
) -> Option<Command> {
    let session_id_str = payload.get("session_id").and_then(|v| v.as_str())?;
    let session_id = SessionId::from(session_id_str.to_owned());

    let message = payload
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("Welcome to nullslop!")
        .to_owned();

    let entry = make_entry(message);
    let push = PushChatEntry { session_id, entry };

    Some(Command::PushChatEntry(push))
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        reason = "test code, panics are acceptable"
    )]

    use super::*;

    #[test]
    fn push_chat_entry_produces_system_entry() {
        // Given a translator.
        let translator = build_translator();

        // When translating a push_chat_entry command.
        let session_id = SessionId::new();
        let payload = serde_json::json!({
            "session_id": session_id.to_string(),
            "message": "Hello from plugin!"
        });
        let result = translator("push_chat_entry", payload);

        // Then a PushChatEntry command is produced.
        let cmd = result.expect("should produce a command");
        match cmd {
            Command::PushChatEntry(pce) => {
                assert_eq!(pce.session_id, session_id);
                assert!(pce.entry.text().contains("Hello from plugin!"));
            }
            other => panic!("expected PushChatEntry, got {other:?}"),
        }
    }

    #[test]
    fn push_chat_entry_transient_produces_transient_entry() {
        // Given a translator.
        let translator = build_translator();

        // When translating a push_chat_entry_transient command.
        let session_id = SessionId::new();
        let payload = serde_json::json!({
            "session_id": session_id.to_string(),
            "message": "New session started."
        });
        let result = translator("push_chat_entry_transient", payload);

        // Then a PushChatEntry with transient entry is produced.
        let cmd = result.expect("should produce a command");
        match cmd {
            Command::PushChatEntry(pce) => {
                assert!(pce.entry.text().contains("New session started."));
            }
            other => panic!("expected PushChatEntry, got {other:?}"),
        }
    }

    #[test]
    fn unknown_command_returns_none() {
        // Given a translator.
        let translator = build_translator();

        // When translating an unknown command.
        let result = translator("nonexistent::cmd", serde_json::json!({}));

        // Then None is returned.
        assert!(result.is_none());
    }

    #[test]
    fn missing_session_id_returns_none() {
        // Given a translator.
        let translator = build_translator();

        // When payload lacks session_id.
        let payload = serde_json::json!({ "message": "hello" });
        let result = translator("push_chat_entry", payload);

        // Then None is returned.
        assert!(result.is_none());
    }

    #[test]
    fn default_message_when_missing() {
        // Given a translator.
        let translator = build_translator();

        // When payload lacks message.
        let session_id = SessionId::new();
        let payload = serde_json::json!({
            "session_id": session_id.to_string()
        });
        let result = translator("push_chat_entry", payload);

        // Then the default message is used.
        let cmd = result.expect("should produce a command");
        match cmd {
            Command::PushChatEntry(pce) => {
                assert!(pce.entry.text().contains("Welcome to nullslop!"));
            }
            other => panic!("expected PushChatEntry, got {other:?}"),
        }
    }
}
