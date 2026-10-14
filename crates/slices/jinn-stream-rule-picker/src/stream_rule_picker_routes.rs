//! The stream-rule picker's route rows and input hook.
//!
//! Fully slice-owned, like every picker: each key the picker responds to is
//! a [`RouteRow`] the slice attaches itself; the kernel contributes no
//! keybind, no scope variant, and no picker identifier.
//!
//! Two shapes are worth knowing:
//!
//! - **Actions** carry a closure that runs against the picker's own cell,
//!   and reach the configuration layer only through [`ActionCtx`] — the
//!   established seam.
//! - **The filter** is an *input hook* rather than rows, because `RouteRow`
//!   has no catch-all variant. Registering the hook makes the composition
//!   keymap synthesize the printable-character catch-all and the editing
//!   keys for this scope automatically, so typing keeps working.
//!
//! A cell guard is never held across a persistence call: the confirm
//! snapshots the pending set, drops the guard, writes through the config
//! layer, then reports the outcome back into the cell.

use std::sync::Arc;

use jinn_preferences_config::StreamRulesStorage;
use jinn_preferences_config::StreamRulesStorageError;
use jinn_slices::KeyRoutes;
use jinn_slices::RouteId;
use jinn_slices::RouteResult as IntentResult;
use jinn_slices::cell::TypedCell;
use jinn_slices::route::{
    ActionCtx, ActionFn, BindSite, EditIntent, InputHook, RouteOutcome, RouteRow, ScopeSignal,
};
use jinn_stream_rule_picker_msg::StreamRulePickerState;
use jinn_stream_rule_picker_msg::stream_rule_picker_scope;

use crate::stream_rule_picker_actions;

/// The picker's cell — the single home for everything it shows.
type StreamRulePickerCell = TypedCell<StreamRulePickerState>;

/// The picker's rows as data: the keys it binds, in footer order.
///
/// The footer renders from this, and a test asserts every entry here is a
/// key the picker actually binds — a footer that advertises a dead key is a
/// bug.
pub const STREAM_RULE_PICKER_BINDINGS: &[(&str, &str)] = &[
    ("<tab>", "toggle"),
    ("<enter>", "confirm"),
    ("<esc>", "cancel"),
];

/// Every key the picker binds in its own scope, for the wiring test.
#[must_use]
pub fn bound_keys() -> Vec<&'static str> {
    STREAM_RULE_PICKER_BINDINGS
        .iter()
        .map(|(key, _)| *key)
        .collect()
}

/// Wraps a picker action in an [`ActionFn`], handing it both the dispatch
/// context and the cell.
fn action<F>(cell: &StreamRulePickerCell, f: F) -> ActionFn
where
    F: Fn(&mut ActionCtx<'_>, &StreamRulePickerCell) -> IntentResult + Send + Sync + 'static,
{
    let cell = cell.clone();
    ActionFn::new(move |mut ctx| f(&mut ctx, &cell))
}

/// Builds one `Action` row binding `key` in the picker's own scope.
fn row(
    action_name: &'static str,
    key: &'static str,
    category: &'static str,
    display: &'static str,
    run: ActionFn,
) -> RouteRow {
    RouteRow {
        route_id: RouteId::new(action_name),
        scope: stream_rule_picker_scope(),
        key,
        category,
        site: BindSite::OwnScope,
        feature: "stream-rule-picker",
        outcome: RouteOutcome::Action {
            action: action_name,
            display,
            run,
        },
    }
}

/// Attaches every row the stream-rule picker owns.
///
/// The opener binds in the `Normal` static scope rather than the picker's
/// own: a key that opens the picker cannot live inside the scope it opens.
/// The remaining rows are `OwnScope`, so they bind only while the picker is
/// on top and cannot shadow another slice's keys.
pub fn attach_stream_rule_picker_rows(routes: &KeyRoutes, cell: &StreamRulePickerCell) {
    routes.attach(RouteRow {
        route_id: RouteId::new("stream-rule-picker:open"),
        scope: stream_rule_picker_scope(),
        key: "<leader>sx",
        category: "general",
        site: BindSite::StaticScopes(&["Normal"]),
        feature: "stream-rule-picker",
        outcome: RouteOutcome::Action {
            action: "open-stream-rule-picker",
            display: "toggle stream rules",
            run: action(cell, open_stream_rule_picker),
        },
    });

    routes.attach(row(
        "toggle-stream-rule-picker",
        "<tab>",
        "general",
        "toggle the highlighted rule",
        action(cell, toggle_highlighted_rule),
    ));
    routes.attach(row(
        "confirm-stream-rule-picker",
        "<enter>",
        "general",
        "save the pending disabled set",
        action(cell, confirm_stream_rule_picker),
    ));
    routes.attach(row(
        "cancel-stream-rule-picker",
        "<esc>",
        "general",
        "close without changing any rule",
        action(cell, cancel_stream_rule_picker),
    ));

    attach_navigation_rows(routes, cell);
}

/// Attaches the four list-navigation rows.
///
/// They cannot be `StaticIntent` rows: composition's `static_intent` table
/// knows only shared-chrome intents, none of them picker navigation, and a
/// row naming an unknown id is silently dropped with a warning. So the
/// picker implements its own navigation, the same shape the skill picker
/// uses.
fn attach_navigation_rows(routes: &KeyRoutes, cell: &StreamRulePickerCell) {
    for (name, key, display, step) in [
        ("move-stream-rule-picker-up", "<up>", "move up", Nav::Up),
        (
            "move-stream-rule-picker-down",
            "<down>",
            "move down",
            Nav::Down,
        ),
        (
            "page-stream-rule-picker-up",
            "<pgup>",
            "page up",
            Nav::PageUp,
        ),
        (
            "page-stream-rule-picker-down",
            "<pgdn>",
            "page down",
            Nav::PageDown,
        ),
    ] {
        routes.attach(row(
            name,
            key,
            "navigation",
            display,
            action(cell, move |ctx, cell| {
                navigate(ctx, cell, step);
                IntentResult::empty()
            }),
        ));
    }
}

/// Which list-navigation key was pressed.
#[derive(Debug, Clone, Copy)]
enum Nav {
    /// `<up>` — one row.
    Up,
    /// `<down>` — one row.
    Down,
    /// `<pgup>` — half the visible window.
    PageUp,
    /// `<pgdn>` — half the visible window.
    PageDown,
}

/// Moves the highlight within the measured viewport.
fn navigate(_ctx: &mut ActionCtx<'_>, cell: &StreamRulePickerCell, nav: Nav) {
    cell.update(|picker| {
        let viewport = picker.results_viewport;
        match nav {
            Nav::Up => picker.selection.move_up(viewport),
            Nav::Down => picker.selection.move_down(viewport),
            Nav::PageUp => picker.selection.page_up(viewport),
            Nav::PageDown => picker.selection.page_down(viewport),
        }
    });
}

/// Registers the picker's filter editing hook.
///
/// The composition keymap turns this registration into the scope's
/// printable-character catch-all plus the editing keys, which is why typing
/// in the filter needs no rows of its own.
pub fn register_stream_rule_picker_input_hook(routes: &KeyRoutes, cell: &StreamRulePickerCell) {
    // The hook outlives this call, so it owns the cell rather than borrowing it.
    let owned = cell.clone();
    let hook: InputHook = Arc::new(move |intent: &EditIntent| {
        owned.update(|picker| match intent {
            EditIntent::InsertChar(ch) => picker.selection.insert_char(*ch),
            EditIntent::DeleteBackward | EditIntent::DeleteForward => {
                picker.selection.backspace();
            }
            EditIntent::CursorLeft => picker.selection.move_cursor_left(),
            EditIntent::CursorRight => picker.selection.move_cursor_right(),
            // The filter is a single-line box: home/end have no meaning here.
            EditIntent::CursorHome | EditIntent::CursorEnd => {}
            EditIntent::Paste(text) => picker.selection.insert_text(text),
        });
        Some(IntentResult::empty())
    });
    routes.register_input_hook(&stream_rule_picker_scope(), hook);
}

// ── Actions ─────────────────────────────────────────────────────────────

/// Opens the picker: seed the rows from the configured rules and push the
/// picker's scope.
///
/// The rule list is launch-global — one `jinn.toml`, every session — so the
/// seed reads the configuration layer the dispatch hands every route
/// action, not any session's state. A malformed section reads as no rules
/// (`unwrap_or_default`), matching how the stream-rules slice itself treats
/// one. The `disabled_rules` array rides the same layer and seeds each
/// row's pending state at the persisted value, so reopening after a
/// confirm shows what the file holds.
fn open_stream_rule_picker(ctx: &mut ActionCtx<'_>, cell: &StreamRulePickerCell) -> IntentResult {
    let rules = ctx
        .config
        .get_list::<jinn_preferences_config::schemas::StreamRuleConfig>()
        .unwrap_or_default();
    let disabled_names: std::collections::HashSet<String> = ctx
        .config
        .get::<jinn_preferences_config::StreamRulesConfig>()
        .unwrap_or_default()
        .disabled_rules
        .into_iter()
        .collect();
    // The theme the rows were last painted with, so a reopen does not
    // recolor under the user's eyes; a first open falls back to the default.
    let theme = cell.read().theme.clone();

    cell.update(|picker| {
        stream_rule_picker_actions::open(picker, &rules, &disabled_names, &theme);
    });

    IntentResult::empty().with_scope_signal(ScopeSignal::Push(stream_rule_picker_scope()))
}

/// `<tab>`: flip the highlighted rule's pending state. The picker stays
/// open and the file is untouched — the write belongs to the confirm.
fn toggle_highlighted_rule(_ctx: &mut ActionCtx<'_>, cell: &StreamRulePickerCell) -> IntentResult {
    cell.update(stream_rule_picker_actions::toggle_highlighted);
    IntentResult::empty()
}

/// `<enter>`: write the pending set through the storage boundary and close.
///
/// Every pending flag is written by name through
/// [`StreamRulesStorage::set_stream_rule_disabled`] — the comment-preserving
/// patch path — so the picker states "rule X is off" rather than
/// re-serializing the section itself. The first failure stops the write and
/// is reported in the picker's status line, with the scope left alone: a
/// failed confirm keeps the picker open over the state the user tried to
/// save, rather than closing as if it had succeeded.
///
/// A rule dropped between open and confirm (the file edited underneath the
/// session) surfaces as [`StreamRulesStorageError::UnknownRule`] and lands
/// in the same status line — never a panic.
///
/// A successful confirm also publishes
/// [`jinn_stream_rules_msg::RefreshStreamRuleMatcher`], which
/// the stream-rules slice's matcher actor answers by recompiling and
/// reinstalling the live matcher — so the confirmed set takes effect for
/// the next stream, not the next launch. The publish rides the result's
/// message list, so a failed confirm (which returns before this point)
/// publishes nothing.
fn confirm_stream_rule_picker(
    ctx: &mut ActionCtx<'_>,
    cell: &StreamRulePickerCell,
) -> IntentResult {
    // Snapshot the pending set and drop the guard before writing: the
    // storage call takes its own locks on the document.
    let pending: Vec<(String, bool)> =
        cell.update(|picker| stream_rule_picker_actions::pending_set(picker));

    let config = ctx.config;
    match write_pending_set(config, &pending) {
        Ok(()) => {
            cell.update(|picker| {
                picker.status = Some(confirmation_status(&pending));
            });
            IntentResult::empty()
                .with_message(jinn_stream_rules_msg::RefreshStreamRuleMatcher)
                .with_scope_signal(ScopeSignal::PopIf(stream_rule_picker_scope()))
        }
        Err(report) => {
            let message = failure_status(report.current_context());
            cell.update(move |picker| picker.status = Some(message));
            IntentResult::empty()
        }
    }
}

/// Writes every pending flag through the config layer, by name.
///
/// Stops at the first failure: a half-applied set is reported, not silently
/// completed — the flags that did land are idempotent on a retry, and the
/// user sees which confirm broke.
fn write_pending_set(
    config: &jinn_config::ConfigLayer,
    pending: &[(String, bool)],
) -> Result<(), error_stack::Report<StreamRulesStorageError>> {
    for (name, disabled) in pending {
        config.set_stream_rule_disabled(name, *disabled)?;
    }
    Ok(())
}

/// The status line a successful confirm leaves: what the set now is, and
/// when it bites.
///
/// "Shortly", not "on restart": the confirm publishes the matcher-refresh
/// command after the write lands, and the stream-rules slice's matcher
/// actor recompiles then — the next stream's rules are the confirmed ones.
/// The refresh is eventual, so the wording claims immediacy only as far as
/// the code guarantees it.
fn confirmation_status(pending: &[(String, bool)]) -> String {
    let disabled: Vec<&str> = pending
        .iter()
        .filter(|(_, disabled)| *disabled)
        .map(|(name, _)| name.as_str())
        .collect();
    if disabled.is_empty() {
        "stream rules: all armed (takes effect shortly)".to_owned()
    } else {
        format!(
            "stream rules: disabled {} (takes effect shortly)",
            disabled.join(", ")
        )
    }
}

/// The status line a failed confirm leaves, naming what went wrong.
fn failure_status(error: &StreamRulesStorageError) -> String {
    match error {
        StreamRulesStorageError::UnknownRule { name } => {
            format!("no stream rule is named `{name}` — the file changed; reopen the picker")
        }
        StreamRulesStorageError::Read { detail } => {
            format!("stream rules could not be read: {detail}")
        }
        StreamRulesStorageError::Write { detail } => {
            format!("stream rules could not be written: {detail}")
        }
    }
}

/// `<esc>`: close without writing anything.
///
/// No cell mutation at all: the pending set simply dies with the picker's
/// next open, which reseeds every row from the file. Cancel cannot leave a
/// partial write behind because it performs no write.
fn cancel_stream_rule_picker(
    _ctx: &mut ActionCtx<'_>,
    _cell: &StreamRulePickerCell,
) -> IntentResult {
    IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(stream_rule_picker_scope()))
}
