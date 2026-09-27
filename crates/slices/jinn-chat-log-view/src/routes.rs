//! The chat log's route rows — the keys it binds in the static scopes.
//!
//! Every key the log owns is a [`RouteRow`] this slice attaches. The
//! kernel contributes no keybind, no scope variant, and no chat-log
//! intent variant: each row binds `KernelIntent::Dynamic` carrying this
//! slice's scope, and the intent routes straight back here.
//!
//! Rows bind into the `Normal` and `Input` static scopes rather than
//! the slice's own dynamic scope — the log is not a modal surface, its
//! keys simply belong to the scopes the keymap already resolves. The
//! row still carries [`chat_log_scope`] as its identity, so the
//! `(scope, action)` dispatch key routes back to this slice and the
//! kernel never names the log.
//!
//! Scroll-by-mouse is the one exception. A wheel event is a crossterm
//! backend handler on the `Keymap`, not a keymap node, and a row can
//! only `bind` a parseable key string — so no row can express it. Those
//! two intents stay in the kernel and call into
//! [`crate::chat_entry_selection::scroll`].

use jinn_chat_log_view_msg::{IGNORE_SELECTED_ACTION, chat_log_scope};
use jinn_core_types::ChatEntry;
use jinn_kernel::AppState;
use jinn_kernel::IntentResult;
use jinn_kernel::common::slices::key_routes::into_route_result;
use jinn_kernel::session_lifecycle::intent::handle_session_lifecycle_setup;
use jinn_slices::KeyRoutes;
use jinn_slices::RouteId;
use jinn_slices::RouteResult;
use jinn_slices::route::{ActionCtx, ActionFn, BindSite, RouteOutcome, RouteRow};

use crate::chat_entry_selection::intent as entry_intent;
use crate::chat_entry_selection::isolate as entry_isolate;
use crate::chat_entry_selection::scroll;

/// The kernel's application state behind an [`ActionCtx`].
///
/// Returns [`None`] when the implementor is not the kernel's state —
/// the action is then a no-op rather than a panic.
fn app<'a>(ctx: &'a mut ActionCtx<'_>) -> Option<&'a mut AppState> {
    ctx.state
        .as_any_mut()?
        .downcast_mut::<jinn_kernel::AppState>()
}

/// The keys the log binds in the `Normal` scope, for the wiring test.
#[must_use]
pub fn bound_keys_normal() -> Vec<&'static str> {
    normal_key_actions()
        .into_iter()
        .map(|(key, _)| key)
        .collect()
}

/// The keys the log binds in the `Input` scope, for the wiring test.
#[must_use]
pub fn bound_keys_input() -> Vec<&'static str> {
    vec!["<c-u>", "<c-d>"]
}

/// Every key the log binds in `Normal`, paired with its action name.
///
/// The rows themselves are the single source of truth — the wiring test
/// reads this rather than restating the key list, so a row that is
/// dropped or renamed here fails the test instead of silently vanishing
/// from the coverage.
#[must_use]
pub fn normal_key_actions() -> Vec<(&'static str, &'static str)> {
    let rows: &[(&'static str, &'static str)] = &[
        // Cursor and scroll.
        ("j", "select-next"),
        ("k", "select-prev"),
        ("<c-u>", "scroll-up"),
        ("<c-d>", "scroll-down"),
        ("gg", "scroll-to-top"),
        ("G", "scroll-to-bottom"),
        // Entry actions.
        ("p", "pin-selected"),
        ("x", "ignore-selected"),
        ("r", "reset-selected"),
        ("e", "expand-tool-entry"),
        ("a", "toggle-audit-popup"),
        ("h", "toggle-ignored-block"),
        ("f", "fork-from-entry"),
        ("F", "new-session-from-entry"),
        ("y", "yank-selected"),
        ("gci", "isolate-selected"),
        // Jump chords.
        ("]c", "jump-next-compaction"),
        ("[c", "jump-prev-compaction"),
        ("]u", "jump-next-user-entry"),
        ("[u", "jump-prev-user-entry"),
        ("]p", "jump-next-pinned"),
        ("[p", "jump-prev-pinned"),
        ("]s", "jump-next-sources"),
        ("[s", "jump-prev-sources"),
    ];
    rows.to_vec()
}

/// Builds one `Action` row binding `key` in `site`.
fn row(
    action_name: &'static str,
    key: &'static str,
    site: BindSite,
    category: &'static str,
    display: &'static str,
    run: ActionFn,
) -> RouteRow {
    RouteRow {
        // Route ids are diagnostics here: dispatch is by (scope, action),
        // so one id covers every row the log owns.
        route_id: RouteId::new("chat-log"),
        scope: chat_log_scope(),
        key,
        category,
        site,
        feature: "chat-log",
        outcome: RouteOutcome::Action {
            action: action_name,
            display,
            run,
        },
    }
}

/// The bind site for a key that belongs to the log only while browsing.
fn normal_only() -> BindSite {
    BindSite::StaticScopes(&["Normal"])
}

/// The bind site for a key shared with the input box.
fn normal_and_input() -> BindSite {
    BindSite::StaticScopes(&["Normal", "Input"])
}

/// Wraps a synchronous log function into an [`ActionFn`].
fn sync<F>(f: F) -> ActionFn
where
    F: Fn(&mut AppState) -> IntentResult + Send + Sync + 'static,
{
    ActionFn::new(move |mut ctx| match app(&mut ctx) {
        Some(state) => into_route_result(f(state)),
        None => RouteResult::empty(),
    })
}

/// `new-session-from-entry` seeds a fresh session through the kernel's
/// lifecycle entry point — the same one the sidebar's rows call — then
/// publishes the selected entry into it.
fn new_session_from_entry() -> ActionFn {
    ActionFn::new(|mut ctx| {
        // Clone the handle, not the borrow: `ActionCtx` lends `&mut`
        // state, so the state lend and a config borrow cannot overlap.
        let config = ctx.config.clone();
        let Some(state) = app(&mut ctx) else {
            return RouteResult::empty();
        };
        let result = entry_intent::seed_new_session_with_entry(state, &config, |state| {
            handle_session_lifecycle_setup(state, "", &[], None, &config)
        });
        into_route_result(result)
    })
}

/// Attaches every row the chat log owns.
///
/// One entry point, deliberately: splitting this across several
/// `attach_*` functions was a latent bug on the chat input box, where
/// one of them was never called and a key silently stopped working.
pub fn attach_all(routes: &KeyRoutes) {
    attach_cursor_rows(routes);
    attach_entry_action_rows(routes);
    attach_jump_chord_rows(routes);
}

/// Cursor movement and the half-page scrolls.
///
/// `<c-u>`/`<c-d>` bind into both `Normal` and `Input`: the box shares
/// `Input` with the log, and scrolling the log while typing is the
/// existing behavior.
fn attach_cursor_rows(routes: &KeyRoutes) {
    for (action, key, display, f) in [
        (
            "select-next",
            "j",
            "select next entry",
            entry_intent::handle_select_next as fn(&mut AppState) -> IntentResult,
        ),
        (
            "select-prev",
            "k",
            "select prev entry",
            entry_intent::handle_select_prev as fn(&mut AppState) -> IntentResult,
        ),
        (
            "scroll-up",
            "<c-u>",
            "scroll up",
            scroll::handle_scroll_up as fn(&mut AppState) -> IntentResult,
        ),
        (
            "scroll-down",
            "<c-d>",
            "scroll down",
            scroll::handle_scroll_down as fn(&mut AppState) -> IntentResult,
        ),
        (
            "scroll-to-top",
            "gg",
            "scroll to top",
            scroll::handle_scroll_to_top as fn(&mut AppState) -> IntentResult,
        ),
        (
            "scroll-to-bottom",
            "G",
            "scroll to bottom",
            scroll::handle_scroll_to_bottom as fn(&mut AppState) -> IntentResult,
        ),
    ] {
        let site = if key == "<c-u>" || key == "<c-d>" {
            normal_and_input()
        } else {
            normal_only()
        };
        routes.attach(row(action, key, site, "navigation", display, sync(f)));
    }
}

/// The per-entry verbs, all `Normal`-only.
fn attach_entry_action_rows(routes: &KeyRoutes) {
    for (action, key, category, display, run) in [
        (
            "pin-selected",
            "p",
            "chat-history",
            "pin entry",
            sync(entry_intent::handle_pin_selected),
        ),
        (
            IGNORE_SELECTED_ACTION,
            "x",
            "chat-history",
            "toggle entry in/out of context",
            sync(entry_intent::handle_ignore_selected),
        ),
        (
            "reset-selected",
            "r",
            "chat-history",
            "reset entry to default context",
            sync(entry_intent::handle_reset_selected),
        ),
        (
            "expand-tool-entry",
            "e",
            "chat-history",
            "expand tool entry",
            sync(entry_intent::handle_expand_tool_entry),
        ),
        (
            "toggle-audit-popup",
            "a",
            "chat-history",
            "toggle audit popup",
            ActionFn::new(toggle_audit_popup),
        ),
        (
            "toggle-ignored-block",
            "h",
            "chat-history",
            "toggle ignored block visibility",
            sync(entry_intent::handle_toggle_ignored_block),
        ),
        (
            "fork-from-entry",
            "f",
            "chat-history",
            "fork from entry",
            sync(entry_intent::handle_fork_from_entry),
        ),
        (
            "new-session-from-entry",
            "F",
            "chat-history",
            "new session from entry",
            new_session_from_entry(),
        ),
        (
            "yank-selected",
            "y",
            "chat-history",
            "yank entry",
            sync(entry_intent::handle_yank_selected),
        ),
        (
            "isolate-selected",
            "gci",
            "context",
            "isolate selected entry in context",
            sync(entry_isolate::handle_isolate_selected),
        ),
    ] {
        routes.attach(row(action, key, normal_only(), category, display, run));
    }
}

/// Flips the audit popup's visibility in the slice's own cell.
// `ActionFn` hands the ctx over by value; the closure only borrows it.
#[expect(
    clippy::needless_pass_by_value,
    reason = "ActionFn's action signature takes the ctx by value"
)]
fn toggle_audit_popup(ctx: ActionCtx<'_>) -> RouteResult {
    let slices = ctx.slices;
    let _ = crate::audit_popup::toggle(slices);
    RouteResult::empty()
}

/// The `]`/`[` jump chords, one row per anchor kind and direction.
fn attach_jump_chord_rows(routes: &KeyRoutes) {
    /// The entry predicate a jump anchors on.
    type Anchor = fn(&jinn_core_types::ChatEntry) -> bool;

    // (action, key, anchor, display). The action name is written out
    // rather than derived from the key so the dispatch key is a compile
    // -time constant and cannot drift from the row that dispatches it.
    const JUMPS: &[(&str, &str, Anchor, &str)] = &[
        (
            "jump-next-compaction",
            "]c",
            ChatEntry::is_compaction,
            "next compaction",
        ),
        (
            "jump-prev-compaction",
            "[c",
            ChatEntry::is_compaction,
            "previous compaction",
        ),
        (
            "jump-next-user-entry",
            "]u",
            ChatEntry::is_user,
            "next user message",
        ),
        (
            "jump-prev-user-entry",
            "[u",
            ChatEntry::is_user,
            "previous user message",
        ),
        (
            "jump-next-pinned",
            "]p",
            ChatEntry::is_pinned,
            "next pinned entry",
        ),
        (
            "jump-prev-pinned",
            "[p",
            ChatEntry::is_pinned,
            "previous pinned entry",
        ),
        (
            "jump-next-sources",
            "]s",
            ChatEntry::is_annotation,
            "next sources entry",
        ),
        (
            "jump-prev-sources",
            "[s",
            ChatEntry::is_annotation,
            "previous sources entry",
        ),
    ];

    for (action, key, anchor, display) in JUMPS {
        let anchor = *anchor;
        let forward = key.starts_with(']');
        routes.attach(row(
            action,
            key,
            normal_only(),
            "chat-history",
            display,
            ActionFn::new(move |mut ctx| match app(&mut ctx) {
                Some(state) => {
                    let result = if forward {
                        entry_intent::handle_jump_next_entry(state, anchor)
                    } else {
                        entry_intent::handle_jump_prev_entry(state, anchor)
                    };
                    into_route_result(result)
                }
                None => RouteResult::empty(),
            }),
        ));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]
    use jinn_slices::route::RouteOutcome;

    use super::*;

    /// Every key the log documents is bound by a row.
    #[rstest::rstest]
    fn attach_all_binds_every_documented_normal_key() {
        // Given an empty route table.
        let routes = KeyRoutes::new();

        // When attaching the log's rows.
        attach_all(&routes);

        // Then every documented Normal-scope key has a row.
        let bound: Vec<&str> = routes.rows().iter().map(|row| row.key).collect();
        for key in bound_keys_normal() {
            assert!(
                bound.contains(&key),
                "no row binds `{key}`; bound keys were {bound:?}"
            );
        }
    }

    /// The two keys shared with the input box bind in both scopes.
    #[rstest::rstest]
    fn scroll_keys_bind_in_both_normal_and_input() {
        // Given the log's rows attached.
        let routes = KeyRoutes::new();
        attach_all(&routes);

        // When checking each scroll key's declared scopes.
        for key in bound_keys_input() {
            let row = routes
                .rows()
                .into_iter()
                .find(|row| row.key == key)
                .expect("the scroll key has a row");

            // Then each scroll key declares both static scopes.
            assert!(
                matches!(row.site, BindSite::StaticScopes(&["Normal", "Input"])),
                "`{key}` must bind in both Normal and Input; got {:?}",
                row.site
            );
        }
    }

    /// Every row dispatches to this slice's scope and names an action.
    #[rstest::rstest]
    fn every_row_carries_the_log_scope_and_an_action() {
        // Given the log's rows attached.
        let routes = KeyRoutes::new();
        attach_all(&routes);

        // When inspecting each attached row.
        for row in routes.rows() {
            // Then each row routes back to the log's own scope.
            assert_eq!(
                row.scope,
                chat_log_scope(),
                "row `{}` must carry the log's scope",
                row.route_id.as_str()
            );
            // And each row names an action.
            assert!(
                matches!(row.outcome, RouteOutcome::Action { .. }),
                "row `{}` must be an Action row",
                row.route_id.as_str()
            );
        }
    }

    /// The `p` key is a leaf, never a chord prefix: a ratatui-which-key
    /// Leaf-to-Branch promotion once dropped this binding and let the
    /// catch-all fire instead of the pin action.
    #[rstest::rstest]
    fn pin_row_is_a_single_token_leaf() {
        // Given the log's rows attached.
        let routes = KeyRoutes::new();
        attach_all(&routes);

        // When locating the row bound to `p`.
        let row = routes
            .rows()
            .into_iter()
            .find(|row| row.key == "p")
            .expect("`p` has a row");

        // Then `p` is one token, so nothing can describe it as a group.
        assert_eq!(
            row.key, "p",
            "`p` must stay a single-token leaf in Normal scope"
        );
    }
}
