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

//! The task-list picker's route rows and input hook.
//!
//! Two behaviors here are specific to this picker and are the reason it could
//! not simply be moved:
//!
//! - **Escape pops only the picker.** The browser is opened from the sidebar's
//!   task-list section and is read-only, so clearing every overlay would drop
//!   the sidebar scope and strand the user in Normal with their sidebar gone.
//!   Every other picker clears the overlay stack. That difference used to be a
//!   branch in the kernel's escape handler keyed on the picker *kind*; it is
//!   now a property this picker declares for itself.
//! - **Enter does nothing.** Task management happens through the task tools, so
//!   a confirm key that appeared to do something would be a lie.

use std::sync::Arc;

use jinn_slices::KeyRoutes;
use jinn_slices::RouteId;
use jinn_slices::RouteResult as IntentResult;
use jinn_slices::cell::TypedCell;
use jinn_slices::route::{
    ActionCtx, ActionFn, BindSite, EditIntent, InputHook, RouteOutcome, RouteRow, ScopeSignal,
};

use crate::task_list_picker_actions;

/// The picker's cell — the single home for everything it shows.
type TaskListPickerCell = TypedCell<jinn_tools_msg::TaskListPickerState>;

/// The picker's rows as data: the keys it binds, in footer order.
///
/// The footer renders from this, and a test asserts every entry is a key the
/// picker actually binds — a footer advertising a dead key is a bug.
pub const TASK_LIST_PICKER_BINDINGS: &[(&str, &str)] =
    &[("<esc>", "close"), ("<enter>", "nothing — read only")];

/// The kernel's application state behind an [`ActionCtx`].
fn app<'a>(ctx: &'a mut ActionCtx<'_>) -> Option<&'a mut jinn_domain::AppState> {
    ctx.state
        .as_any_mut()?
        .downcast_mut::<jinn_domain::AppState>()
}

/// Wraps a picker action in an [`ActionFn`], handing it the cell.
fn action<F>(cell: &TaskListPickerCell, f: F) -> ActionFn
where
    F: Fn(&mut ActionCtx<'_>, &TaskListPickerCell) -> IntentResult + Send + Sync + 'static,
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
        scope: jinn_tools_msg::task_list_picker_scope(),
        key,
        category,
        site: BindSite::OwnScope,
        feature: "tools",
        outcome: RouteOutcome::Action {
            action: route_id,
            display,
            run,
        },
    }
}

/// Attaches every row the task-list picker owns.
pub fn attach_task_list_picker_rows(routes: &KeyRoutes, cell: &TaskListPickerCell) {
    // The opener is deliberately absent: this menu is opened by the
    // sidebar's own `s` key, which calls `task_list_picker_opener` in
    // this slice. It claims no leader chord, matching trunk.
    routes.attach(row(
        "confirm-task-list-picker",
        "<enter>",
        "general",
        "nothing — this browser is read-only",
        action(cell, confirm_task_list_picker),
    ));
    routes.attach(row(
        "quit-task-list-picker",
        "<esc>",
        "general",
        "close the browser",
        action(cell, close_task_list_picker),
    ));
    routes.attach(row(
        "new-session-from-task-list-picker",
        "<c-n>",
        "general",
        "start a new session",
        action(cell, new_session),
    ));
    routes.attach(row(
        "clear-filter-or-leave-task-list-picker",
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
/// knows only six route ids, none of them picker intents, and a row naming an
/// unknown id is silently dropped with a warning.
fn attach_navigation_rows(routes: &KeyRoutes, cell: &TaskListPickerCell) {
    for (name, key, display, step) in [
        ("move-task-list-picker-up", "<up>", "move up", Nav::Up),
        (
            "move-task-list-picker-down",
            "<down>",
            "move down",
            Nav::Down,
        ),
        ("page-task-list-picker-up", "<pgup>", "page up", Nav::PageUp),
        (
            "page-task-list-picker-down",
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
pub fn register_task_list_picker_input_hook(routes: &KeyRoutes, cell: &TaskListPickerCell) {
    let owned = cell.clone();
    let hook: InputHook = Arc::new(move |intent: &EditIntent| {
        owned.update(|picker| match intent {
            EditIntent::InsertChar(ch) => picker.tree.insert_char(*ch),
            EditIntent::DeleteBackward | EditIntent::DeleteForward => picker.tree.backspace(),
            EditIntent::CursorLeft => picker.tree.move_cursor_left(),
            EditIntent::CursorRight => picker.tree.move_cursor_right(),
            // The filter is a single-line box: home/end have no meaning here.
            EditIntent::CursorHome | EditIntent::CursorEnd => {}
            EditIntent::Paste(text) => picker.tree.insert_text(text),
        });
        Some(IntentResult::empty())
    });
    routes.register_input_hook(&jinn_tools_msg::task_list_picker_scope(), hook);
}

// ── Actions ─────────────────────────────────────────────────────────────

/// Opens the browser over the active session's current task list.
/// The task-list picker's opener, resolved against the cell at dispatch time.
///
/// The sidebar's `s` key needs this without holding a `TypedCell` handle: it
/// is a separate slice, and handing out the picker cell would leak the
/// picker's internals across the boundary. Looking the cell up in the shared
/// registry by its public slot keeps the sidebar ignorant of everything but
/// the fact that the tools slice owns a task list browser.
#[must_use]
pub fn task_list_opener_action() -> jinn_slices::route::ActionFn {
    jinn_slices::route::ActionFn::new(|mut ctx| {
        let Some(cell) = ctx.slices.reader(&jinn_tools_msg::task_list_picker_slot()) else {
            return IntentResult::empty();
        };
        open_task_list_picker(&mut ctx, &cell)
    })
}

fn open_task_list_picker(ctx: &mut ActionCtx<'_>, cell: &TaskListPickerCell) -> IntentResult {
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    let theme = state.frontend.theme.clone();
    let list = state.active_session().task_list().clone();
    cell.update(|picker| task_list_picker_actions::open(picker, &list, &theme));
    IntentResult::empty()
        .with_scope_signal(ScopeSignal::Push(jinn_tools_msg::task_list_picker_scope()))
}

/// Escape: pop **only** the picker.
///
/// A plain `Pop` would be the normal picker behavior, but this browser is
/// reached from the sidebar: dropping the whole overlay stack would take the
/// sidebar with it. Popping one scope leaves the user where they were.
fn close_task_list_picker(_ctx: &mut ActionCtx<'_>, _cell: &TaskListPickerCell) -> IntentResult {
    IntentResult::empty()
        .with_scope_signal(ScopeSignal::PopIf(jinn_tools_msg::task_list_picker_scope()))
}

/// Enter: deliberately nothing.
///
/// Tasks are changed through the task tools, not by picking a row. The key is
/// still bound so the footer can say the browser is read-only, rather than
/// leaving the user to discover it by pressing a key that does nothing.
fn confirm_task_list_picker(_ctx: &mut ActionCtx<'_>, _cell: &TaskListPickerCell) -> IntentResult {
    IntentResult::empty()
}

/// Starts a new session, as the key does from any picker.
fn new_session(ctx: &mut ActionCtx<'_>, _cell: &TaskListPickerCell) -> IntentResult {
    let config = ctx.config;
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    jinn_domain::feat::session::intent::handle_session_new(state, config)
}

/// Ctrl-C: clear the filter, or close when it is already empty.
fn clear_filter_or_leave(_ctx: &mut ActionCtx<'_>, cell: &TaskListPickerCell) -> IntentResult {
    let mut empty = false;
    cell.update(|picker| {
        if picker.tree.filter().is_empty() {
            empty = true;
        } else {
            picker.tree.backspace();
            // Ctrl-C clears the whole filter, not one character.
            while !picker.tree.filter().is_empty() {
                picker.tree.backspace();
            }
        }
    });
    if empty {
        return IntentResult::empty()
            .with_scope_signal(ScopeSignal::PopIf(jinn_tools_msg::task_list_picker_scope()));
    }
    IntentResult::empty()
}
