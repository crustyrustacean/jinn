//! The saved-attendants picker's route rows and input hook.
//!
//! The opener binds `<leader>sa` in the `Normal` static scope rather than
//! the picker's own: a key that opens a picker cannot live inside the scope
//! it opens. Every other key binds in the picker's own dynamic scope, and
//! the filter is an input hook — `RouteRow` has no catch-all variant, so
//! registering the hook is what synthesizes the printable-character
//! catch-all and the editing keys for this scope.

use std::sync::Arc;

use jinn_attendant_msg::{AttendantSavedPickerState, attendant_saved_picker_scope};
use jinn_preferences_config::schemas::AttendantEntryConfig;
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
type SavedPickerCell = TypedCell<AttendantSavedPickerState>;

/// The picker's rows as data: the keys it binds, in footer order.
///
/// The footer renders from this, so it cannot advertise a dead key.
pub const SAVED_PICKER_BINDINGS: &[(&str, &str)] = &[
    ("<enter>", "create"),
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
        scope: attendant_saved_picker_scope(),
        key,
        category,
        site: BindSite::OwnScope,
        feature: "attendant",
        outcome: RouteOutcome::Action {
            action: route_id,
            display,
            run,
        },
    }
}

/// Attaches every row the saved-attendants picker owns.
pub fn attach_saved_picker_rows(routes: &KeyRoutes, cell: &SavedPickerCell) {
    routes.attach(RouteRow {
        route_id: RouteId::new("attendant:open-saved-picker"),
        scope: attendant_saved_picker_scope(),
        key: "<leader>sa",
        category: "general",
        site: BindSite::StaticScopes(&["Normal"]),
        feature: "attendant",
        outcome: RouteOutcome::Action {
            action: "open-attendant-saved-picker",
            display: "search saved attendants",
            run: action(cell, open_saved_picker),
        },
    });

    routes.attach(row(
        "confirm-attendant-saved-picker",
        "<enter>",
        "general",
        "create this attendant on the active session",
        action(cell, confirm_saved_picker),
    ));
    routes.attach(row(
        "cancel-attendant-saved-picker",
        "<esc>",
        "general",
        "close without creating an attendant",
        action(cell, cancel_saved_picker),
    ));
    routes.attach(row(
        "clear-filter-or-leave-attendant-saved-picker",
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
        ("move-attendant-saved-picker-up", "<up>", "move up", Nav::Up),
        (
            "move-attendant-saved-picker-down",
            "<down>",
            "move down",
            Nav::Down,
        ),
        (
            "page-attendant-saved-picker-up",
            "<pgup>",
            "page up",
            Nav::PageUp,
        ),
        (
            "page-attendant-saved-picker-down",
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
    routes.register_input_hook(&attendant_saved_picker_scope(), filter_input_hook(cell));
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
/// to the document must show up on the next `<leader>sa` without a
/// restart, and a cell that cached its entries at boot could not.
///
/// A document this slice cannot parse opens an empty picker *and says so
/// in the session's log*. The entries are hand-editable, so a typo is
/// likely, and an empty list is indistinguishable from "you have not saved
/// any" — a silent read failure would have the user hunting for a save
/// that happened.
fn open_saved_picker(ctx: &mut ActionCtx<'_>, cell: &SavedPickerCell) -> IntentResult {
    let entries = match ctx.config.get_list::<AttendantEntryConfig>() {
        Ok(entries) => entries,
        Err(error) => {
            tracing::warn!(err = ?error, "failed to read the saved attendants");
            if let Some(state) = app(ctx) {
                state.push_session_error(
                    "Could not read saved attendants from jinn.toml — the [[attendant.entry]] list could not be parsed. See the log for the offending entry.",
                );
            }
            Vec::new()
        }
    };
    let summaries = saved_picker_actions::summaries_of(&entries);
    cell.update(|picker| saved_picker_actions::open(picker, summaries));
    IntentResult::empty().with_scope_signal(ScopeSignal::Push(attendant_saved_picker_scope()))
}

/// `<enter>`: create the highlighted saved attendant on the active session.
fn confirm_saved_picker(ctx: &mut ActionCtx<'_>, cell: &SavedPickerCell) -> IntentResult {
    let Some(name) = cell
        .read()
        .selection
        .selected_item()
        .map(|item| item.entry().name.clone())
    else {
        return IntentResult::empty();
    };
    // The entry is re-read from the live document rather than carried in
    // the cell: the cell holds a display summary, and creating from a
    // summary would be creating from a rendering.
    // The re-read reports like the open does: an entry that listed fine a
    // moment ago and does not now was edited under the popup, and creating
    // nothing without a word would look like the picker is broken.
    let entries = match ctx.config.get_list::<AttendantEntryConfig>() {
        Ok(entries) => entries,
        Err(error) => {
            tracing::warn!(err = ?error, "failed to re-read the saved attendants");
            if let Some(state) = app(ctx) {
                state.push_session_error(
                    "Could not read jinn.toml to create that attendant — the [[attendant.entry]] list could not be parsed.",
                );
            }
            return IntentResult::empty();
        }
    };
    let Some(entry) = entries.into_iter().find(|entry| entry.name == name) else {
        return IntentResult::empty();
    };
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    let Some(created) = crate::saved_create::create_in_state(state, &entry) else {
        return IntentResult::empty();
    };
    IntentResult::empty()
        .with_scope_signal(ScopeSignal::PopIf(attendant_saved_picker_scope()))
        .merge(created)
}

/// `<esc>`: leave without creating an attendant.
fn cancel_saved_picker(_ctx: &mut ActionCtx<'_>, cell: &SavedPickerCell) -> IntentResult {
    cell.update(|picker| picker.selection.reset());
    IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(attendant_saved_picker_scope()))
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
            .with_scope_signal(ScopeSignal::PopIf(attendant_saved_picker_scope()));
    }
    IntentResult::empty()
}
