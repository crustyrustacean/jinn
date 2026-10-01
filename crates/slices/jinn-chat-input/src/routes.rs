//! The chat input's route rows — the keys it binds explicitly.
//!
//! Everything the box binds in its own scope is a [`RouteRow`] this slice
//! attaches. The kernel contributes no keybind, no scope variant, and no
//! chat-input intent variant for these: each row's action dispatches
//! straight into this slice's own handlers.
//!
//! Every key the box owns is a row here. The rows bind into the `Input`
//! scope — the scope the keymap resolves typing in — and because
//! `bind_route_rows` runs after the kernel's static binds, a row replaces
//! the kernel binding for the same key. Printable characters are the one
//! exception a row cannot express; they arrive through the keymap's
//! `Scope::Input` catch-all, which dispatches to `insert-char`.

use jinn_chat_input_msg::chat_input_scope;
use jinn_kernel::AppState;
use jinn_kernel::IntentResult;
use jinn_kernel::common::slices::key_routes::into_route_result;
use jinn_slices::KeyRoutes;
use jinn_slices::RouteId;
use jinn_slices::RouteResult;
use jinn_slices::route::{ActionCtx, ActionFn, BindSite, RouteOutcome, RouteRow};

use crate::intent;

/// The kernel's application state behind an [`ActionCtx`].
///
/// Returns [`None`] when the implementor is not the kernel's state — the
/// action is then a no-op rather than a panic.
fn app<'a>(ctx: &'a mut ActionCtx<'_>) -> Option<&'a mut jinn_kernel::AppState> {
    ctx.state
        .as_any_mut()?
        .downcast_mut::<jinn_kernel::AppState>()
}

/// The editing keys the box binds in the `Input` scope, for the wiring test.
#[must_use]
pub fn bound_keys() -> Vec<&'static str> {
    vec![
        "<enter>",
        "<tab>",
        "<esc>",
        "<c-k>",
        "<M-q>",
        "<s-enter>",
        "<c-enter>",
    ]
}

/// Builds one `Action` row binding `key` in the `Input` scope.
///
/// The box's rows bind `StaticScopes(&["Input"])` rather than
/// `OwnScope`: `FocusScope::Input` remains the focus scope that selects
/// the box, and `Scope::Input` is what the keymap resolves keys in. The
/// row still carries the box's own scope as its identity, so the dynamic
/// intent routes back here and the kernel never names the box.
fn row(
    action_name: &'static str,
    key: &'static str,
    category: &'static str,
    display: &'static str,
    run: ActionFn,
) -> RouteRow {
    RouteRow {
        route_id: RouteId::new(action_name),
        scope: chat_input_scope(),
        key,
        category,
        site: BindSite::StaticScopes(&["Input"]),
        feature: "chat-input",
        outcome: RouteOutcome::Action {
            action: action_name,
            display,
            run,
        },
    }
}

/// Attaches every row the chat input box owns.
pub fn attach_chat_input_rows(routes: &KeyRoutes) {
    routes.attach(row(
        "submit-message",
        "<enter>",
        "general",
        "send this message",
        ActionFn::new(|mut ctx| {
            // Read the layer before the state borrow: `app` takes `ctx`
            // mutably.
            let config = ctx.config;
            let Some(state) = app(&mut ctx) else {
                return RouteResult::empty();
            };
            into_route_result(intent::handle_submit_message(state, config))
        }),
    ));

    routes.attach(row(
        "confirm-autocomplete",
        "<tab>",
        "general",
        "accept the highlighted suggestion",
        ActionFn::new(|mut ctx| {
            let Some(state) = app(&mut ctx) else {
                return RouteResult::empty();
            };
            into_route_result(intent::handle_autocomplete_confirm(state))
        }),
    ));

    routes.attach(row(
        "leave-input-mode",
        "<esc>",
        "general",
        "leave insert mode",
        ActionFn::new(|mut ctx| {
            let config = ctx.config;
            let Some(state) = app(&mut ctx) else {
                return RouteResult::empty();
            };
            into_route_result(intent::handle_enter_normal_mode(state, config))
        }),
    ));

    routes.attach(row(
        "leave-input-mode-alt",
        "<c-k>",
        "general",
        "leave insert mode",
        ActionFn::new(|mut ctx| {
            let config = ctx.config;
            let Some(state) = app(&mut ctx) else {
                return RouteResult::empty();
            };
            into_route_result(intent::handle_enter_normal_mode(state, config))
        }),
    ));

    routes.attach(row(
        "toggle-submission-mode",
        "<M-q>",
        "input",
        "toggle input mode",
        ActionFn::new(|mut ctx| {
            let Some(state) = app(&mut ctx) else {
                return RouteResult::empty();
            };
            into_route_result(intent::handle_toggle_input_mode(state))
        }),
    ));

    // The two alternate-submit keys insert a literal newline rather than
    // sending: they are the box's only multi-line affordance.
    for (name, key) in [
        ("insert-newline-shift", "<s-enter>"),
        ("insert-newline-ctrl", "<c-enter>"),
    ] {
        routes.attach(row(
            name,
            key,
            "input",
            "insert a newline",
            ActionFn::new(|mut ctx| {
                let Some(state) = app(&mut ctx) else {
                    return RouteResult::empty();
                };
                into_route_result(intent::handle_insert_char('\n', state))
            }),
        ));
    }
}

/// Attaches the row bound in the `Normal` static scope.
///
/// `enter-insert-mode` belongs to `Normal` because a key that *enters* the
/// box cannot live inside the box: its own scope is not focused yet. The
/// kernel used to bind `i` and `<c-j>` statically; those binds are the
/// slice's now.
pub fn attach_enter_insert_row(routes: &KeyRoutes) {
    for key in ["i", "<c-j>"] {
        routes.attach(RouteRow {
            route_id: RouteId::new("enter-input-mode"),
            scope: chat_input_scope(),
            key,
            category: "input",
            site: BindSite::StaticScopes(&["Normal"]),
            feature: "chat-input",
            outcome: RouteOutcome::Action {
                action: "enter-insert-mode",
                display: "type a message",
                run: ActionFn::new(|mut ctx| {
                    let Some(state) = app(&mut ctx) else {
                        return RouteResult::empty();
                    };
                    into_route_result(intent::handle_enter_insert_mode(state))
                }),
            },
        });
    }
}

/// Attaches every keybind the box owns, and registers its key hook.
///
/// This is the slice's single entry point for keybinds. `activate` calls
/// this and nothing else, so a row added to the box cannot be forgotten at
/// activation: the one function that binds keys owns all of them.
///
/// Splitting this across several `attach_*` functions was a latent bug —
/// one of them was never called, and the only way into the box (`i` /
/// `<c-j>` in Normal) silently stopped working. Any further split must
/// keep every piece reachable from here.
pub fn attach_all(routes: &KeyRoutes) {
    attach_chat_input_rows(routes);
    attach_enter_insert_row(routes);
    attach_editing_rows(routes);
    attach_insert_char_row(routes);
    attach_paste_text_row(routes);
    crate::key_hook::register(routes);
}

/// Attaches every editing-key action the box owns.
///
/// These carry the keys the kernel used to bind statically into
/// `Scope::Input`. `bind_route_rows` runs after `keymap::init` and a
/// later bind for the same key+scope replaces the earlier one, so these
/// rows take the box's editing keys over from the kernel.
/// The box's editing keys: deletion and cursor motion.
///
/// Each entry names the handler, the key it is bound to, and the which-key
/// label. The handler is resolved by name at dispatch time through
/// `editing_handler`, so the table below stays a table instead of ten
/// near-identical closures.
const EDITING_KEYS: &[(&str, &str, &str)] = &[
    ("delete-backward", "<backspace>", "delete back"),
    ("delete-forward", "<delete>", "delete forward"),
    ("move-cursor-left", "<left>", "move cursor left"),
    ("move-cursor-right", "<right>", "move cursor right"),
    ("move-cursor-home", "<home>", "move to line start"),
    ("move-cursor-end", "<end>", "move to line end"),
    ("move-cursor-up", "<up>", "move cursor up"),
    ("move-cursor-down", "<down>", "move cursor down"),
    ("move-word-left", "<c-left>", "move word left"),
    ("move-word-right", "<c-right>", "move word right"),
];

/// Attaches the box's editing rows.
pub fn attach_editing_rows(routes: &KeyRoutes) {
    for &(name, key, display) in EDITING_KEYS {
        // The action name is captured, not read off the ctx: `ActionCtx`
        // carries no back-reference to the intent that dispatched it.
        let run = match editing_handler(name) {
            Some(handle) => ActionFn::new(move |mut ctx| {
                let Some(state) = app(&mut ctx) else {
                    return RouteResult::empty();
                };
                handle(state)
            }),
            None => ActionFn::new(|_ctx| RouteResult::empty()),
        };
        routes.attach(row(name, key, "input", display, run));
    }
}

/// Resolves an editing action name to its handler.
///
/// An unlisted action yields `None` and its row becomes inert, which keeps
/// a stale keybind from panicking the key path.
fn editing_handler(action: &str) -> Option<fn(&mut AppState) -> IntentResult> {
    let handle: fn(&mut AppState) -> IntentResult = match action {
        "delete-backward" => intent::handle_delete_grapheme,
        "delete-forward" => intent::handle_delete_grapheme_forward,
        "move-cursor-left" => intent::handle_move_cursor_left,
        "move-cursor-right" => intent::handle_move_cursor_right,
        "move-cursor-home" => intent::handle_move_cursor_to_start,
        "move-cursor-end" => intent::handle_move_cursor_to_end,
        "move-cursor-up" => intent::handle_move_cursor_up,
        "move-cursor-down" => intent::handle_move_cursor_down,
        "move-word-left" => intent::handle_move_cursor_word_left,
        "move-word-right" => intent::handle_move_cursor_word_right,
        _ => return None,
    };
    Some(handle)
}

/// Reads back the character an `insert-char` intent's payload carries.
///
/// Returns [`None`] when the payload is not exactly one UTF-8 character,
/// so a malformed intent is dropped rather than inserting a replacement
/// character.
#[must_use]
pub fn char_from_bytes(bytes: &[u8]) -> Option<char> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut chars = text.chars();
    let first = chars.next()?;
    chars.next().is_none().then_some(first)
}

/// Attaches the catch-all row for printable characters.
///
/// A row binds one key, so a row cannot express "every character". The
/// keymap's own `Scope::Input` catch-all carries the character to the
/// slice's `insert-char` action instead (see
/// `keymap_gen::bind_chat_input_catch_all`), which keeps the dispatch a
/// dynamic intent like every other chat input key.
pub fn register_insert_char_catch_all() {
    // The catch-all is installed by the keymap generator, which owns the
    // binding tree; nothing to attach here.
}

/// The action name the character catch-all dispatches to.
pub const INSERT_CHAR_ACTION: &str = "insert-char";

/// Attaches the `insert-char` row the character catch-all dispatches to.
pub fn attach_insert_char_row(routes: &KeyRoutes) {
    routes.attach(row(
        INSERT_CHAR_ACTION,
        "",
        "input",
        "type a character",
        ActionFn::new(|mut ctx| {
            // The catch-all carried the character in the intent's byte
            // payload, which reaches the action as `key_bytes`.
            let Some(ch) = char_from_bytes(&ctx.key_bytes) else {
                return RouteResult::empty();
            };
            let Some(state) = app(&mut ctx) else {
                return RouteResult::empty();
            };
            into_route_result(intent::handle_insert_char(ch, state))
        }),
    ));
}

/// The action name a bracketed paste dispatches to.
///
/// Re-exported from the crossing crate: the kernel mints the same
/// intent name while routing a paste to whichever surface holds focus.
pub use jinn_chat_input_msg::PASTE_TEXT_ACTION;

/// Attaches the `paste-text` row.
///
/// A paste has no key, so it gets no route row binding — the handler
/// mints the dynamic intent from the terminal's bracketed-paste event and
/// hands it here with the text in the byte payload.
pub fn attach_paste_text_row(routes: &KeyRoutes) {
    routes.attach(row(
        PASTE_TEXT_ACTION,
        "",
        "input",
        "paste text",
        ActionFn::new(|mut ctx| {
            let Some(text) = text_from_bytes(&ctx.key_bytes) else {
                return RouteResult::empty();
            };
            let Some(state) = app(&mut ctx) else {
                return RouteResult::empty();
            };
            into_route_result(intent::handle_paste_text(&text, state))
        }),
    ));
}

/// Reads back the text a paste carries in its byte payload.
///
/// UTF-8 that is not valid is dropped: a paste that cannot be decoded has
/// nothing well-defined to insert.
#[must_use]
pub fn text_from_bytes(bytes: &[u8]) -> Option<String> {
    std::str::from_utf8(bytes)
        .ok()
        .map(std::borrow::ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]

    use super::*;

    #[rstest::rstest]
    fn attach_chat_input_rows_registers_the_documented_keys() {
        // Given an empty route table.
        let routes = KeyRoutes::new();

        // When attaching the box's rows.
        attach_chat_input_rows(&routes);
        attach_enter_insert_row(&routes);

        // Then every documented key is bound in the box's scope.
        let bound: Vec<&'static str> = routes
            .rows()
            .into_iter()
            .filter(|row| row.scope == chat_input_scope() && !row.key.is_empty())
            .map(|row| row.key)
            .collect();
        for key in bound_keys() {
            assert!(
                bound.contains(&key),
                "row for `{key}` missing; bound keys were {bound:?}"
            );
        }
    }

    #[rstest::rstest]
    fn every_editing_key_is_bound_by_a_row() {
        // Given an empty route table with the box's editing rows attached.
        let routes = KeyRoutes::new();
        attach_editing_rows(&routes);

        // When collecting the keys every row binds.
        // Then each key the kernel used to bind statically has a row.
        let bound: Vec<&'static str> = routes.rows().into_iter().map(|row| row.key).collect();
        for key in [
            "<backspace>",
            "<delete>",
            "<left>",
            "<right>",
            "<home>",
            "<end>",
            "<up>",
            "<down>",
            "<c-left>",
            "<c-right>",
        ] {
            assert!(bound.contains(&key), "no row binds {key}");
        }
    }

    #[rstest::rstest]
    fn rows_bind_into_the_input_scope() {
        // Given the box's rows attached.
        let routes = KeyRoutes::new();
        attach_chat_input_rows(&routes);
        attach_editing_rows(&routes);

        // When reading each row's declared bind site.
        // Then every row declares the Input static scope, so composition
        // binds it where the keymap resolves typing.
        for row in routes.rows() {
            assert!(
                matches!(row.site, BindSite::StaticScopes(&["Input"])),
                "row {} declares the wrong bind site",
                row.route_id.as_str()
            );
        }
    }

    #[rstest::rstest]
    fn insert_char_action_is_reachable_by_name() {
        // Given the character row attached.
        let routes = KeyRoutes::new();
        attach_insert_char_row(&routes);

        // When collecting the delivered action names.
        // Then the keymap catch-all's action name has a delivering row.
        let action = routes
            .rows()
            .into_iter()
            .find_map(|row| match row.outcome {
                RouteOutcome::Action { action, .. } => Some(action),
                RouteOutcome::StaticIntent(_) => None,
            })
            .expect("the insert-char row is attached");
        assert_eq!(action, INSERT_CHAR_ACTION);
    }

    #[rstest::rstest]
    fn char_from_bytes_reads_a_single_character() {
        // Given a UTF-8 character payload.
        // When decoding it.
        // Then the character is read out.
        assert_eq!(char_from_bytes("é".as_bytes()), Some('é'));
    }

    #[rstest::rstest]
    fn char_from_bytes_rejects_a_multi_character_payload() {
        // Given a payload holding more than one character.
        // When decoding it.
        // Then nothing is read.
        assert_eq!(char_from_bytes(b"ab"), None);
    }

    #[rstest::rstest]
    fn text_from_bytes_reads_a_paste_payload() {
        // Given a multi-line paste payload.
        // When decoding it.
        // Then the whole payload is read as text.
        assert_eq!(
            text_from_bytes("hello\nworld".as_bytes()),
            Some("hello\nworld".to_owned())
        );
    }

    #[rstest::rstest]
    fn text_from_bytes_rejects_invalid_utf8() {
        // Given a payload that is not valid UTF-8.
        // When decoding it.
        // Then nothing is read.
        assert_eq!(text_from_bytes(&[0xff, 0xfe]), None);
    }
}
