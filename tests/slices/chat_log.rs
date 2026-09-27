//! The chat log's keys in the composed keymap.
//!
//! The log's keys used to be static kernel binds producing chat-log
//! `KernelIntent` variants. They are now route rows the log's slice
//! attaches, so each key resolves to `KernelIntent::Dynamic` carrying
//! the log's slice id and an action name. These tests pin that
//! resolution against the real composed keymap — the static binds plus
//! every slice's rows — because that composition is the only place the
//! two can collide.

use jinn_chat_log_view_msg::chat_log_scope;
use jinn_kernel::{KernelIntent, Key, KeyEvent, Modifiers};
use jinn_tui::Scope;
use jinn_tui::app::WhichKeyInstance;

use crate::common::composed_keymap;

/// A keymap at `scope` with every slice's rows attached, as launch does.
fn wk(scope: Scope) -> WhichKeyInstance {
    WhichKeyInstance::new(composed_keymap(), scope)
}

/// A plain (unmodified) character key.
fn plain(ch: char) -> KeyEvent {
    KeyEvent {
        key: Key::Char(ch),
        modifiers: Modifiers::none(),
    }
}

/// The chat-log slice and action a resolved intent dispatches to.
fn as_chat_log(intent: &KernelIntent) -> (String, String) {
    match intent {
        KernelIntent::Dynamic(dynamic) => (dynamic.slice.key(), dynamic.action.clone()),
        other => panic!("expected a slice route action, got {other:?}"),
    }
}

/// Feeds a key path of already-built events.
fn resolve_events(scope: Scope, path: &[KeyEvent]) -> Option<KernelIntent> {
    let mut instance = wk(scope);
    for key in path {
        if let Some(intent) = instance.handle_key(key.clone()) {
            return Some(intent);
        }
    }
    None
}

/// A `<c-x>` key event.
fn ctrl(ch: char) -> KeyEvent {
    KeyEvent {
        key: Key::Char(ch),
        modifiers: Modifiers {
            ctrl: true,
            ..Modifiers::none()
        },
    }
}

/// Every key the log owns in Normal scope dispatches to the log's slice.
#[rstest::rstest]
fn every_normal_key_resolves_to_a_chat_log_action() {
    // Given the composed keymap.
    // When resolving each key the log binds in Normal scope.
    for (key, path) in event_paths() {
        let intent = resolve_events(Scope::Normal, &path)
            .unwrap_or_else(|| panic!("`{key}` must resolve in Normal scope"));
        let (slice, action) = as_chat_log(&intent);

        // Then it dispatches to the chat log's slice.
        assert_eq!(
            slice,
            chat_log_scope().key(),
            "`{key}` must resolve to the chat-log slice; got {slice} ({action})"
        );
    }
}

/// The log's rows paired with the key path the keymap consumes for each.
fn normal_key_actions() -> Vec<(&'static str, &'static str)> {
    jinn_chat_log_view::routes::normal_key_actions()
}

/// The event path the keymap consumes for each key the log binds.
fn event_paths() -> Vec<(String, Vec<KeyEvent>)> {
    let paths: Vec<(String, Vec<KeyEvent>)> = vec![
        ("j".into(), vec![plain('j')]),
        ("k".into(), vec![plain('k')]),
        ("<c-u>".into(), vec![ctrl('u')]),
        ("<c-d>".into(), vec![ctrl('d')]),
        ("gg".into(), vec![plain('g'), plain('g')]),
        ("G".into(), vec![plain('G')]),
        ("p".into(), vec![plain('p')]),
        ("x".into(), vec![plain('x')]),
        ("r".into(), vec![plain('r')]),
        ("e".into(), vec![plain('e')]),
        ("a".into(), vec![plain('a')]),
        ("h".into(), vec![plain('h')]),
        ("f".into(), vec![plain('f')]),
        ("F".into(), vec![plain('F')]),
        ("y".into(), vec![plain('y')]),
        ("gci".into(), vec![plain('g'), plain('c'), plain('i')]),
        ("]c".into(), vec![plain(']'), plain('c')]),
        ("[c".into(), vec![plain('['), plain('c')]),
        ("]u".into(), vec![plain(']'), plain('u')]),
        ("[u".into(), vec![plain('['), plain('u')]),
        ("]p".into(), vec![plain(']'), plain('p')]),
        ("[p".into(), vec![plain('['), plain('p')]),
        ("]s".into(), vec![plain(']'), plain('s')]),
        ("[s".into(), vec![plain('['), plain('s')]),
    ];
    paths
}

/// Each Normal-scope key carries the action name its row publishes.
#[rstest::rstest]
fn normal_keys_resolve_to_their_documented_actions() {
    // Given the composed keymap.
    // Look the path up by key name so a reordering cannot mispair them.
    let paths = event_paths();
    for (key, expected) in normal_key_actions() {
        let path = paths
            .iter()
            .find(|(name, _)| name == key)
            .unwrap_or_else(|| panic!("no event path for `{key}`"))
            .1
            .clone();
        // When resolving the key and reading its action name.
        let (_, action) = as_chat_log(&resolve_events(Scope::Normal, &path).expect("resolves"));

        // Then the action is the one the row advertises.
        assert_eq!(action, expected, "`{key}` must dispatch `{expected}`");
    }
}

/// The two scroll keys work while the user is typing.
#[rstest::rstest]
fn scroll_keys_resolve_in_the_input_scope() {
    // Given the composed keymap.
    // When resolving the scroll keys in Input scope.
    for (key, path) in [("<c-u>", vec![ctrl('u')]), ("<c-d>", vec![ctrl('d')])] {
        let intent = resolve_events(Scope::Input, &path)
            .unwrap_or_else(|| panic!("`{key}` must resolve in Input scope"));
        let (slice, _) = as_chat_log(&intent);

        // Then they dispatch to the chat log's slice.
        assert_eq!(
            slice,
            chat_log_scope().key(),
            "`{key}` in Input scope must reach the chat log; got {slice}"
        );
    }
}

/// The `p` key is a leaf, never a chord prefix.
///
/// Regression: ratatui-which-key promoted the Normal-scope `p` leaf to
/// a branch when another scope described `p` as a group, which dropped
/// the pin binding and let the catch-all fire instead.
#[rstest::rstest]
fn pin_key_is_a_leaf_and_not_a_chord_prefix() {
    // Given the composed keymap.
    // When pressing `p` in Normal scope.
    let intent = wk(Scope::Normal).handle_key(plain('p'));

    // Then it fires the pin action, not a branch or a no-op.
    let (_, action) = as_chat_log(&intent.expect("`p` must resolve"));
    assert_eq!(action, "pin-selected", "`p` must still fire the pin action");
}

/// The jump chords keep their which-key group labels.
///
/// The chords are now leaf rows, but the group *labels* still come from
/// the keymap, so the which-key popup must keep showing them.
#[rstest::rstest]
fn jump_chord_groups_keep_their_labels_in_normal_scope() {
    // Given the composed keymap.
    let keymap = composed_keymap();

    // When collecting the descriptions Normal scope shows.
    let groups = keymap.bindings_for_scope(Scope::Normal);
    let all_desc: Vec<&str> = groups
        .iter()
        .flat_map(|g| g.bindings.iter().map(|b| b.description.as_str()))
        .collect();

    // Then both chord groups are still described.
    assert!(
        all_desc.iter().any(|d| d.contains("next")),
        "the next-chord group should appear in Normal scope; got {all_desc:?}"
    );
    assert!(
        all_desc.iter().any(|d| d.contains("previous")),
        "the previous-chord group should appear in Normal scope; got {all_desc:?}"
    );
}

/// The jump chords stay out of the input scope.
#[rstest::rstest]
fn jump_chords_do_not_dispatch_in_the_input_scope() {
    // Given the composed keymap.
    // When pressing `]` in Input scope.
    let intent = wk(Scope::Input).handle_key(plain(']'));

    // Then it is not one of the log's jump actions.
    if let Some(KernelIntent::Dynamic(dynamic)) = intent {
        assert_ne!(
            dynamic.action, "jump-next-compaction",
            "`]` in Input scope must not fire a jump chord"
        );
    }
}
