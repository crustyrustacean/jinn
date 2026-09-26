// Copyright (C) 2026 Jayson Lennon
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! The session picker's route rows and input hook.

use jinn_slices::KeyRoutes;
use jinn_slices::RouteId;
use jinn_slices::RouteResult as IntentResult;
use jinn_slices::cell::TypedCell;
use jinn_slices::route::{
    ActionCtx, ActionFn, BindSite, EditIntent, InputHook, RouteOutcome, RouteRow, ScopeSignal,
};

use crate::session_picker_actions;

/// The picker's cell — the single home for everything it shows.
type SessionPickerCell = TypedCell<jinn_session_store_msg::SessionPickerState>;

/// The picker's rows as data: the keys it binds, in footer order.
pub const SESSION_PICKER_BINDINGS: &[(&str, &str)] = &[
    ("<esc>", "close"),
    ("<enter>", "open session"),
    ("<c-n>", "new session"),
];

/// The kernel's application state behind an [`ActionCtx`].
fn app<'a>(ctx: &'a mut ActionCtx<'_>) -> Option<&'a mut jinn_domain::AppState> {
    ctx.state
        .as_any_mut()?
        .downcast_mut::<jinn_domain::AppState>()
}

/// Wraps a picker action in an [`ActionFn`], handing it the cell.
fn action<F>(cell: &SessionPickerCell, f: F) -> ActionFn
where
    F: Fn(&mut ActionCtx<'_>, &SessionPickerCell) -> IntentResult + Send + Sync + 'static,
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
        scope: jinn_session_store_msg::session_picker_scope(),
        key,
        category,
        site: BindSite::OwnScope,
        feature: "session-store",
        outcome: RouteOutcome::Action {
            action: route_id,
            display,
            run,
        },
    }
}

/// Attaches every row the session picker owns.
pub fn attach_session_picker_rows(routes: &KeyRoutes, cell: &SessionPickerCell) {
    routes.attach(RouteRow {
        route_id: RouteId::new("session-picker:open"),
        scope: jinn_session_store_msg::session_picker_scope(),
        key: "<leader>ss",
        category: "general",
        site: BindSite::StaticScopes(&["Normal", "Sidebar"]),
        feature: "session-store",
        outcome: RouteOutcome::Action {
            action: "open-session-picker",
            display: "search sessions",
            run: action(cell, open_session_picker),
        },
    });

    routes.attach(row(
        "close-session-picker",
        "<esc>",
        "general",
        "close the browser",
        action(cell, close_session_picker),
    ));
    routes.attach(row(
        "confirm-session-picker",
        "<enter>",
        "general",
        "open the highlighted session",
        action(cell, confirm_session_picker),
    ));
    routes.attach(row(
        "new-session-from-session-picker",
        "<c-n>",
        "general",
        "start a new session",
        action(cell, new_session),
    ));
    routes.attach(row(
        "clear-filter-or-leave-session-picker",
        "<c-c>",
        "general",
        "clear the filter, or close when already empty",
        action(cell, clear_filter_or_leave),
    ));

    attach_navigation_rows(routes, cell);
}

/// Attaches the four list-navigation rows.
///
/// Not `StaticIntent` rows: composition's `static_intent` table knows only
/// six route ids, none of them picker intents, and a row naming an unknown id
/// is silently dropped.
fn attach_navigation_rows(routes: &KeyRoutes, cell: &SessionPickerCell) {
    for (name, key, display, step) in [
        ("move-session-picker-up", "<up>", "move up", Nav::Up),
        ("move-session-picker-down", "<down>", "move down", Nav::Down),
        ("page-session-picker-up", "<pgup>", "page up", Nav::PageUp),
        (
            "page-session-picker-down",
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
                        Nav::Up => picker.tree.move_up(viewport),
                        Nav::Down => picker.tree.move_down(viewport),
                        Nav::PageUp => picker.tree.page_up(viewport),
                        Nav::PageDown => picker.tree.page_down(viewport),
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
pub fn register_session_picker_input_hook(routes: &KeyRoutes, cell: &SessionPickerCell) {
    let owned = cell.clone();
    let hook: InputHook = std::sync::Arc::new(move |intent: &EditIntent| {
        owned.update(|picker| match intent {
            EditIntent::InsertChar(ch) => picker.tree.insert_char(*ch),
            EditIntent::DeleteBackward | EditIntent::DeleteForward => picker.tree.backspace(),
            EditIntent::CursorLeft => picker.tree.move_cursor_left(),
            EditIntent::CursorRight => picker.tree.move_cursor_right(),
            EditIntent::CursorHome | EditIntent::CursorEnd => {}
            EditIntent::Paste(text) => picker.tree.insert_text(text),
        });
        Some(IntentResult::empty())
    });
    routes.register_input_hook(&jinn_session_store_msg::session_picker_scope(), hook);
}

// ── Actions ─────────────────────────────────────────────────────────────

/// Opens the browser: reset, push the scope, and ask the actor for the rows.
///
/// The history read is async, so the open does not wait for it — the popup
/// appears with an empty tree and fills in when the actor publishes.
fn open_session_picker(_ctx: &mut ActionCtx<'_>, cell: &SessionPickerCell) -> IntentResult {
    cell.update(session_picker_actions::reset);
    IntentResult::new_message(jinn_session_store_msg::LoadSessionPickerEntries).with_scope_signal(
        ScopeSignal::Push(jinn_session_store_msg::session_picker_scope()),
    )
}

/// Escape: pop the picker.
fn close_session_picker(_ctx: &mut ActionCtx<'_>, _cell: &SessionPickerCell) -> IntentResult {
    IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(
        jinn_session_store_msg::session_picker_scope(),
    ))
}

/// Enter: begin loading the highlighted session and switch to it.
///
/// Nothing highlighted is a no-op rather than an error: the tree is empty
/// while the history read is in flight, and Enter in that window must not
/// close the picker or switch to a session the user never chose.
fn confirm_session_picker(ctx: &mut ActionCtx<'_>, cell: &SessionPickerCell) -> IntentResult {
    let session_id = {
        let guard = cell.read();
        session_picker_actions::highlighted_session(&guard)
    };
    let Some(session_id) = session_id else {
        return IntentResult::empty();
    };
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    state.session.begin_load(session_id.clone());
    IntentResult::new_message(jinn_session_store_msg::SessionLoadRequested { session_id })
        .with_scope_signal(ScopeSignal::PopIf(
            jinn_session_store_msg::session_picker_scope(),
        ))
}

/// Ctrl-N: start a new session.
fn new_session(ctx: &mut ActionCtx<'_>, _cell: &SessionPickerCell) -> IntentResult {
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    jinn_domain::feat::session::intent::handle_session_new(state)
}

/// Ctrl-C: clear the filter, or close when it is already empty.
fn clear_filter_or_leave(_ctx: &mut ActionCtx<'_>, cell: &SessionPickerCell) -> IntentResult {
    let mut empty = false;
    cell.update(|picker| {
        if picker.tree.filter().is_empty() {
            empty = true;
        } else {
            while !picker.tree.filter().is_empty() {
                picker.tree.backspace();
            }
        }
    });
    if empty {
        return IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(
            jinn_session_store_msg::session_picker_scope(),
        ));
    }
    IntentResult::empty()
}

/// The row that opens the browser, exposed so a test can assert its shape.
#[must_use]
pub fn session_picker_open_row() -> RouteRow {
    RouteRow {
        route_id: RouteId::new("session-picker:open"),
        scope: jinn_session_store_msg::session_picker_scope(),
        key: "<leader>ss",
        category: "general",
        site: BindSite::StaticScopes(&["Normal", "Sidebar"]),
        feature: "session-store",
        outcome: RouteOutcome::StaticIntent(RouteId::new("open-session-picker")),
    }
}
