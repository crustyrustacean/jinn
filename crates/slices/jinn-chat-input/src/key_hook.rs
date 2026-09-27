//! The box's raw-key capture hook.
//!
//! `Scope::Input` needs a catch-all for printable characters, but a
//! catch-all built from the shared `EditIntent` vocabulary is a *single-line*
//! edit model: it has no submit, no mode toggle, and no autocomplete
//! navigation. The box is multi-line with its own editing model, so it binds
//! its own catch-all and dispatches characters as a slice-owned action.
//!
//! This hook is installed on the box's scope, not `Scope::Input`, so
//! `bind_route_rows` synthesizes the `Input`-scope catch-all that routes
//! here. The hook returns `None` for any key it does not own, which lets the
//! next matcher run — a declining hook never swallows a keystroke.

use jinn_chat_input_msg::chat_input_scope;
use jinn_slices::DynamicIntent;
use jinn_slices::KeyEvent;
use jinn_slices::KeyHook;
use jinn_slices::KeyRoutes;
#[cfg(test)]
use jinn_slices::Modifiers;

use crate::routes::INSERT_CHAR_ACTION;

/// Builds the box's key hook: raw keys become the box's dynamic intents.
///
/// Every key the box does not own resolves to `None`, so the key falls
/// through to whatever matcher runs next. This is what keeps a chrome key
/// (`<c-c>`, `<f1>`) working while the box has input capture.
#[must_use]
pub fn chat_input_key_hook() -> KeyHook {
    std::sync::Arc::new(|event: &KeyEvent| intent_for_key(event))
}

/// Registers the box's key hook on its scope.
pub fn register(routes: &KeyRoutes) {
    routes.register_key_hook(&chat_input_scope(), chat_input_key_hook());
}

/// Maps a raw key to the box's intent, or `None` if the box does not own it.
///
/// # Errors
///
/// None. An unmapped key yields `None` and falls through.
fn intent_for_key(event: &KeyEvent) -> Option<DynamicIntent> {
    use jinn_slices::Key;

    let ctrl = event.modifiers.ctrl;
    let action = match event.key {
        // A printable character, with no ctrl/alt held. Alt is excluded so
        // chords like `<M-q>` (mode toggle) keep reaching their rows.
        Key::Char(ch) if !ctrl && !event.modifiers.alt => {
            return Some(DynamicIntent::with_bytes(
                chat_input_scope(),
                INSERT_CHAR_ACTION,
                "type a character",
                ch.to_string().into_bytes(),
            ));
        }
        Key::Backspace => "delete-backward",
        Key::Delete => "delete-forward",
        Key::Left if ctrl => "move-word-left",
        Key::Right if ctrl => "move-word-right",
        Key::Left => "move-cursor-left",
        Key::Right => "move-cursor-right",
        Key::Home => "move-cursor-home",
        Key::End => "move-cursor-end",
        // Up/Down double as autocomplete navigation when the popup is open
        // and as cursor motion when it is closed; the handler owns that
        // branch.
        Key::Up => "move-cursor-up",
        Key::Down => "move-cursor-down",
        _ => return None,
    };
    Some(DynamicIntent::new(chat_input_scope(), action, action))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, reason = "test code")]

    use super::*;
    use jinn_slices::Key;

    /// An unmapped key is declined so the next matcher runs.
    #[rstest::rstest]
    #[case(Key::F(1))]
    #[case(Key::PageUp)]
    #[case(Key::Tab)]
    fn unmapped_keys_are_declined(#[case] key: Key) {
        // Given a key the box does not own.
        let event = KeyEvent {
            key: key.clone(),
            modifiers: Modifiers::none(),
        };

        // When mapping it.
        let intent = intent_for_key(&event);

        // Then the hook declines, letting the key fall through.
        assert!(
            intent.is_none(),
            "{key:?} is not the box's; the hook must decline it",
        );
    }

    /// A printable character becomes an `insert-char` intent carrying the char.
    #[rstest::rstest]
    #[test]
    fn printable_char_becomes_insert_char() {
        // Given an unmodified printable character.
        let event = KeyEvent {
            key: Key::Char('a'),
            modifiers: Modifiers::none(),
        };

        // When mapping it.
        let intent = intent_for_key(&event).expect("a printable char is the box's");

        // Then it is an `insert-char` action for the box's scope.
        assert_eq!(intent.action, INSERT_CHAR_ACTION);
        // And the character travels in the payload.
        assert_eq!(String::from_utf8(intent.bytes).ok().as_deref(), Some("a"));
    }

    /// Ctrl+Arrow is word-wise motion, not single-character motion.
    #[rstest::rstest]
    #[test]
    fn ctrl_arrow_is_word_motion() {
        // Given a Ctrl+Left press.
        let event = KeyEvent {
            key: Key::Left,
            modifiers: Modifiers::ctrl(),
        };

        // When mapping it.
        let intent = intent_for_key(&event).expect("Ctrl+Left is the box's");

        // Then it is word-wise motion.
        assert_eq!(intent.action, "move-word-left");
    }

    /// A bare arrow key is single-character motion.
    #[rstest::rstest]
    #[test]
    fn bare_arrow_is_character_motion() {
        // Given a bare Left press.
        let event = KeyEvent {
            key: Key::Left,
            modifiers: Modifiers::none(),
        };

        // When mapping it.
        let intent = intent_for_key(&event).expect("Left is the box's");

        // Then it is single-character motion.
        assert_eq!(intent.action, "move-cursor-left");
    }
}
