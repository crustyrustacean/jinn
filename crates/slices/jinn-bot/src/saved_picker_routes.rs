//! The saved-bots picker's route rows and input hook.
//!
//! The opener binds `<leader>sb` in the `Normal` static scope rather than
//! the picker's own: a key that opens a picker cannot live inside the scope
//! it opens. Every other key binds in the picker's own dynamic scope, and
//! the filter is an input hook — `RouteRow` has no catch-all variant, so
//! registering the hook is what synthesizes the printable-character
//! catch-all and the editing keys for this scope.

use std::sync::Arc;

use jinn_bot_msg::{BotSavedPickerState, bot_saved_picker_scope};
use jinn_preferences_config::schemas::BotEntryConfig;
use jinn_slices::KeyRoutes;
use jinn_slices::RouteId;
use jinn_slices::RouteResult as IntentResult;
use jinn_slices::cell::TypedCell;
use jinn_slices::route::{
    ActionCtx, ActionFn, BindSite, EditIntent, InputHook, RouteOutcome, RouteRow, ScopeSignal,
    SliceActionState,
};

use crate::saved_picker_actions;

/// The picker's cell — the single home for everything it shows.
type SavedPickerCell = TypedCell<BotSavedPickerState>;

/// The picker's rows as data: the keys it binds, in footer order.
///
/// The footer renders from this, so it cannot advertise a dead key.
pub const SAVED_PICKER_BINDINGS: &[(&str, &str)] = &[
    ("<enter>", "attach"),
    ("<c-a>", "attach & keep open"),
    ("<c-enter>", "spawn standalone"),
    ("<esc>", "close"),
    ("<c-c>", "clear filter or close"),
];

/// The kernel's application state behind an [`ActionCtx`].
fn app<'a>(ctx: &'a mut ActionCtx<'_>) -> Option<&'a mut jinn_kernel::AppState> {
    ctx.state
        .as_any_mut()?
        .downcast_mut::<jinn_kernel::AppState>()
}

/// Wraps a picker action in an [`ActionFn`], handing it the cell.
fn action<F>(cell: &SavedPickerCell, f: F) -> ActionFn
where
    F: Fn(&mut ActionCtx<'_>, &SavedPickerCell) -> IntentResult + Send + Sync + 'static,
{
    let cell = cell.clone();
    ActionFn::new(move |mut ctx| f(&mut ctx, &cell))
}

/// Builds an own-scope row bound to a slice action.
fn row(
    route_id: &'static str,
    key: &'static str,
    category: &'static str,
    display: &'static str,
    run: ActionFn,
) -> RouteRow {
    RouteRow {
        route_id: RouteId::new(route_id),
        scope: bot_saved_picker_scope(),
        key,
        category,
        site: BindSite::OwnScope,
        feature: "bot",
        outcome: RouteOutcome::Action {
            action: route_id,
            display,
            run,
        },
    }
}

/// Attaches every row the saved-bots picker owns.
pub fn attach_saved_picker_rows(routes: &KeyRoutes, cell: &SavedPickerCell) {
    routes.attach(RouteRow {
        route_id: RouteId::new("bot:open-saved-picker"),
        scope: bot_saved_picker_scope(),
        key: "<leader>sb",
        category: "general",
        site: BindSite::StaticScopes(&["Normal"]),
        feature: "bot",
        outcome: RouteOutcome::Action {
            action: "open-bot-saved-picker",
            display: "search saved bots",
            run: action(cell, open_saved_picker),
        },
    });

    routes.attach(row(
        "confirm-bot-saved-picker",
        "<enter>",
        "general",
        "attach this bot to the active session and close",
        action(cell, confirm_saved_picker),
    ));
    routes.attach(row(
        "attach-bot-saved-picker-keeping-it-open",
        "<c-a>",
        "general",
        "attach this bot to the active session and keep the picker open",
        action(cell, attach_and_keep_open),
    ));
    routes.attach(row(
        "spawn-standalone-bot-from-saved-entry",
        "<c-enter>",
        "general",
        "spawn this bot standalone (no parent) and close",
        action(cell, confirm_standalone),
    ));
    routes.attach(row(
        "cancel-bot-saved-picker",
        "<esc>",
        "general",
        "close without attaching a bot",
        action(cell, cancel_saved_picker),
    ));
    routes.attach(row(
        "clear-filter-or-leave-bot-saved-picker",
        "<c-c>",
        "general",
        "clear the filter, or close when already empty",
        action(cell, clear_filter_or_leave),
    ));

    attach_navigation_rows(routes, cell);
}

/// Attaches the four list-navigation rows.
///
/// They cannot be `StaticIntent` rows: composition's `static_intent` table
/// knows only six route ids, none of them picker intents, and a row naming
/// an unknown id is silently dropped with a warning.
fn attach_navigation_rows(routes: &KeyRoutes, cell: &SavedPickerCell) {
    for (name, key, display, step) in [
        ("move-bot-saved-picker-up", "<up>", "move up", Nav::Up),
        (
            "move-bot-saved-picker-down",
            "<down>",
            "move down",
            Nav::Down,
        ),
        ("page-bot-saved-picker-up", "<pgup>", "page up", Nav::PageUp),
        (
            "page-bot-saved-picker-down",
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
            action(cell, move |_ctx, cell| {
                cell.update(|picker| {
                    let viewport = picker.results_viewport;
                    match step {
                        Nav::Up => picker.selection.move_up(viewport),
                        Nav::Down => picker.selection.move_down(viewport),
                        Nav::PageUp => picker.selection.page_up(viewport),
                        Nav::PageDown => picker.selection.page_down(viewport),
                    }
                });
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

/// Registers the picker's filter editing hook.
///
/// The composition keymap turns this registration into the scope's
/// printable-character catch-all plus the editing keys.
pub fn register_saved_picker_input_hook(routes: &KeyRoutes, cell: &SavedPickerCell) {
    routes.register_input_hook(&bot_saved_picker_scope(), filter_input_hook(cell));
}

/// The filter's editing hook, exposed so a test can type into the filter
/// without going through a keymap.
pub(crate) fn filter_input_hook(cell: &SavedPickerCell) -> InputHook {
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
    hook
}

/// Opens the picker over whatever `jinn.toml` holds *now*.
///
/// The read happens here, at open, rather than at activation: a hand edit
/// to the document must show up on the next `<leader>sb` without a
/// restart, and a cell that cached its entries at boot could not.
///
/// A document this slice cannot parse opens an empty picker *and says so
/// in the session's log*. The entries are hand-editable, so a typo is
/// likely, and an empty list is indistinguishable from "you have not saved
/// any" — a silent read failure would have the user hunting for a save
/// that happened.
fn open_saved_picker(ctx: &mut ActionCtx<'_>, cell: &SavedPickerCell) -> IntentResult {
    let entries = match ctx.config.get_list::<BotEntryConfig>() {
        Ok(entries) => entries,
        Err(error) => {
            tracing::warn!(err = ?error, "failed to read the saved bots");
            if let Some(state) = app(ctx) {
                state.push_session_error(
                    "Could not read saved bots from jinn.toml — the [[bot.entry]] list could not be parsed. See the log for the offending entry.",
                );
            }
            Vec::new()
        }
    };
    let summaries = saved_picker_actions::summaries_of(&entries);
    cell.update(|picker| saved_picker_actions::open(picker, summaries));
    IntentResult::empty().with_scope_signal(ScopeSignal::Push(bot_saved_picker_scope()))
}

/// `<enter>`: attach the highlighted saved bot to the active session
/// and close the picker.
///
/// Attaching leaves the active session where it was — the parent keeps the
/// conversation, and the new bot is reported in its log rather than
/// replacing it.
fn confirm_saved_picker(ctx: &mut ActionCtx<'_>, cell: &SavedPickerCell) -> IntentResult {
    let Some(attached) = attach_highlighted(ctx, cell) else {
        return IntentResult::empty();
    };
    IntentResult::empty()
        .with_scope_signal(ScopeSignal::PopIf(bot_saved_picker_scope()))
        .merge(attached)
}

/// `<c-a>`: attach the highlighted saved bot and leave the picker
/// exactly as it was.
///
/// Attaching several in a row is the point: every entry is pre-saved, so
/// loading a set of them is one gesture per entry rather than a reopen per
/// entry. Nothing about the picker changes — not the filter, not the
/// highlight, not the scope stack — so the next entry is one key away.
fn attach_and_keep_open(ctx: &mut ActionCtx<'_>, cell: &SavedPickerCell) -> IntentResult {
    attach_highlighted(ctx, cell).unwrap_or_else(IntentResult::empty)
}

/// Creates the highlighted entry's bot under the active session,
/// reading the entry from the live document.
///
/// `None` means nothing was created: no entry is highlighted, the document
/// no longer lists the highlighted one, or the active session is gone or is
/// itself a bot.
fn attach_highlighted(ctx: &mut ActionCtx<'_>, cell: &SavedPickerCell) -> Option<IntentResult> {
    let name = cell
        .read()
        .selection
        .selected_item()
        .map(|item| item.entry().name.clone())?;
    // The entry is re-read from the live document rather than carried in
    // the cell: the cell holds a display summary, and creating from a
    // summary would be creating from a rendering.
    // The re-read reports like the open does: an entry that listed fine a
    // moment ago and does not now was edited under the popup, and creating
    // nothing without a word would look like the picker is broken.
    let entries = match ctx.config.get_list::<BotEntryConfig>() {
        Ok(entries) => entries,
        Err(error) => {
            tracing::warn!(err = ?error, "failed to re-read the saved bots");
            if let Some(state) = app(ctx) {
                state.push_session_error(
                    "Could not read jinn.toml to create that bot — the [[bot.entry]] list could not be parsed.",
                );
            }
            return None;
        }
    };
    let entry = entries.into_iter().find(|entry| entry.name == name)?;
    let state = app(ctx)?;
    crate::saved_create::create_in_state(state, &entry)
}

/// `<c-enter>`: spawn the highlighted saved bot standalone, switch to it,
/// and close the picker.
///
/// The entry's configuration is honored whole — including its `prep_mode`,
/// so an entry saved composing arrives composing — but no parent link is
/// formed: the active session supplies the environment only, and the
/// spawned bot's trigger is locked to `manual`. There is no is-bot refusal
/// on this path, because there is no lineage to confuse. The spawn lands
/// the user on the bot it made — unlike `<enter>`, which attaches in the
/// background and leaves the session the picker was opened from active.
fn confirm_standalone(ctx: &mut ActionCtx<'_>, cell: &SavedPickerCell) -> IntentResult {
    let Some(spawned) = spawn_standalone_highlighted(ctx, cell) else {
        return IntentResult::empty();
    };
    IntentResult::empty()
        .with_scope_signal(ScopeSignal::PopIf(bot_saved_picker_scope()))
        .merge(spawned)
}

/// Creates the highlighted entry's bot with no parent, reading the entry
/// from the live document.
///
/// `None` means nothing was created: no entry is highlighted, the document
/// no longer lists the highlighted one, or the active session is gone.
fn spawn_standalone_highlighted(
    ctx: &mut ActionCtx<'_>,
    cell: &SavedPickerCell,
) -> Option<IntentResult> {
    let name = cell
        .read()
        .selection
        .selected_item()
        .map(|item| item.entry().name.clone())?;
    // The entry is re-read from the live document rather than carried in
    // the cell, exactly as the attach path does: the cell holds a display
    // summary, and creating from a summary would be creating from a
    // rendering.
    let entries = match ctx.config.get_list::<BotEntryConfig>() {
        Ok(entries) => entries,
        Err(error) => {
            tracing::warn!(err = ?error, "failed to re-read the saved bots");
            if let Some(state) = app(ctx) {
                state.push_session_error(
                    "Could not read jinn.toml to create that bot — the [[bot.entry]] list could not be parsed.",
                );
            }
            return None;
        }
    };
    let entry = entries.into_iter().find(|entry| entry.name == name)?;
    let state = app(ctx)?;
    crate::saved_create::create_standalone_in_state(state, &entry)
}

/// `<esc>`: leave without creating a bot.
fn cancel_saved_picker(_ctx: &mut ActionCtx<'_>, cell: &SavedPickerCell) -> IntentResult {
    cell.update(|picker| picker.selection.reset());
    IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(bot_saved_picker_scope()))
}

/// `<c-c>`: clear the filter, or leave when it is already empty.
fn clear_filter_or_leave(_ctx: &mut ActionCtx<'_>, cell: &SavedPickerCell) -> IntentResult {
    let filter_empty = cell.update(|picker| {
        if picker.selection.filter().is_empty() {
            true
        } else {
            picker.selection.clear_filter();
            false
        }
    });
    if filter_empty {
        return IntentResult::empty()
            .with_scope_signal(ScopeSignal::PopIf(bot_saved_picker_scope()));
    }
    IntentResult::empty()
}
