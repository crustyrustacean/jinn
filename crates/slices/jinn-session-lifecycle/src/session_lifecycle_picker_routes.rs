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

//! The session-lifecycle picker's route rows and input hook.
//!
//! Every key the picker responds to is a [`RouteRow`] this slice attaches
//! itself; the kernel contributes no keybind, no scope variant, and no picker
//! identifier.
//!
//! - **Actions** carry a closure that runs against the picker's own cell and
//!   reach app state only through [`SliceActionState::as_any_mut`] — the same
//!   seam `jinn-sidebar`, `jinn-term`, `jinn-project`, and `jinn-preferences`
//!   already use.
//! - **The filter** is an *input hook* rather than rows, because `RouteRow`
//!   has no catch-all variant. Registering the hook makes the composition
//!   keymap synthesize the printable-character catch-all and the editing keys
//!   for this scope, so typing keeps working.
//!
//! A cell guard is never held across an await: every action snapshots what it
//! needs, mutates, and drops the guard before it returns.

use std::sync::Arc;

use jinn_preferences_config::schemas::SessionLifecycle;
use jinn_session_lifecycle_msg::picker_state::SessionLifecyclePickerState;
use jinn_session_lifecycle_msg::{
    ArgInputState, arg_input_scope, arg_input_slot, session_lifecycle_picker_scope,
};
use jinn_slices::FocusScope;
use jinn_slices::KeyRoutes;
use jinn_slices::RouteId;
use jinn_slices::RouteResult as IntentResult;
use jinn_slices::cell::TypedCell;
use jinn_slices::route::{
    ActionCtx, ActionFn, BindSite, EditIntent, InputHook, RouteOutcome, RouteRow, ScopeEnterHook,
    ScopeSignal,
};

use crate::session_lifecycle_picker_actions;

/// The picker's cell — the single home for everything it shows.
type LifecyclePickerCell = TypedCell<SessionLifecyclePickerState>;

/// The picker's rows as data: the keys it binds, in footer order.
///
/// The footer renders from this, and a test asserts every entry is a key the
/// picker actually binds — a footer advertising a dead key is a bug.
pub const SESSION_LIFECYCLE_PICKER_BINDINGS: &[(&str, &str)] =
    &[("<enter>", "start"), ("<esc>", "cancel")];

/// The kernel's application state behind an [`ActionCtx`].
///
/// Confirming a lifecycle changes the session, so it downcasts. When the state
/// is not the kernel's (a test double), the action declines rather than
/// panicking.
fn app<'a>(ctx: &'a mut ActionCtx<'_>) -> Option<&'a mut jinn_domain::AppState> {
    ctx.state
        .as_any_mut()?
        .downcast_mut::<jinn_domain::AppState>()
}

/// Wraps a picker action in an [`ActionFn`], handing it both the dispatch
/// context and the cell.
fn action<F>(cell: &LifecyclePickerCell, f: F) -> ActionFn
where
    F: Fn(&mut ActionCtx<'_>, &LifecyclePickerCell) -> IntentResult + Send + Sync + 'static,
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
        scope: session_lifecycle_picker_scope(),
        key,
        category,
        site: BindSite::OwnScope,
        feature: "session-lifecycle",
        outcome: RouteOutcome::Action {
            action: route_id,
            display,
            run,
        },
    }
}

/// Attaches every row the session-lifecycle picker owns.
///
/// The opener binds in the `Normal` static scope rather than the picker's own:
/// a key that opens the picker cannot live inside the scope it opens. The
/// project picker's `<c-enter>` also routes here, so a project can hand off to
/// this menu without either picker naming the other.
pub fn attach_session_lifecycle_picker_rows(routes: &KeyRoutes, cell: &LifecyclePickerCell) {
    routes.attach(RouteRow {
        route_id: RouteId::new("session-lifecycle:open"),
        scope: session_lifecycle_picker_scope(),
        key: "<leader>sl",
        category: "general",
        site: BindSite::StaticScopes(&["Normal"]),
        feature: "session-lifecycle",
        outcome: RouteOutcome::Action {
            action: "open-session-lifecycle-picker",
            display: "search session-lifecycle",
            run: action(cell, open_session_lifecycle_picker),
        },
    });

    routes.attach(row(
        "confirm-session-lifecycle-picker",
        "<enter>",
        "general",
        "start the highlighted lifecycle",
        action(cell, confirm_session_lifecycle_picker),
    ));
    routes.attach(row(
        "cancel-session-lifecycle-picker",
        "<esc>",
        "general",
        "close the picker",
        action(cell, cancel_session_lifecycle_picker),
    ));
    routes.attach(row(
        "new-session-from-lifecycle-picker",
        "<c-n>",
        "general",
        "start a new session",
        action(cell, new_session),
    ));
    routes.attach(row(
        "clear-filter-or-leave-lifecycle-picker",
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
/// unknown id is silently dropped with a warning. So the picker implements its
/// own navigation.
fn attach_navigation_rows(routes: &KeyRoutes, cell: &LifecyclePickerCell) {
    for (name, key, display, step) in [
        ("move-lifecycle-picker-up", "<up>", "move up", Nav::Up),
        (
            "move-lifecycle-picker-down",
            "<down>",
            "move down",
            Nav::Down,
        ),
        ("page-lifecycle-picker-up", "<pgup>", "page up", Nav::PageUp),
        (
            "page-lifecycle-picker-down",
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
/// printable-character catch-all plus the editing keys, which is why typing in
/// the filter needs no rows of its own.
pub fn register_session_lifecycle_picker_input_hook(
    routes: &KeyRoutes,
    cell: &LifecyclePickerCell,
) {
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
    routes.register_input_hook(&session_lifecycle_picker_scope(), hook);
}

/// Registers the picker's scope-enter hook: the one place its per-open
/// state is built.
///
/// Every opener — this slice's `<leader>sl` row and the project picker's
/// `<c-enter>` — does nothing but request the transition, so both land on the
/// same fresh menu: filter cleared, highlight back at the top, rows rebuilt
/// from configuration. The rows come from the configuration layer and the
/// theme from app state, both reachable through the context the kernel lends
/// at the push.
pub fn register_session_lifecycle_picker_enter_hook(
    routes: &KeyRoutes,
    cell: &LifecyclePickerCell,
) {
    // The hook outlives this call, so it owns the cell rather than borrowing it.
    let owned = cell.clone();
    let hook: ScopeEnterHook = Arc::new(move |mut ctx: ActionCtx<'_>| {
        // Read the config into an owned list before borrowing state: `ctx` lends
        // app state mutably, and holding both borrows at once overlaps.
        let lifecycles = ctx
            .config
            .get_list::<SessionLifecycle>()
            .unwrap_or_default();
        // The rows must be seeded even when the state is not the kernel's (a
        // test double), so only the theme falls back in that case.
        let theme = app(&mut ctx).map_or_else(jinn_theme::default_theme, |state| {
            state.frontend.theme.clone()
        });
        owned.update(|picker| session_lifecycle_picker_actions::open(picker, &lifecycles, &theme));
    });
    routes.register_scope_enter_hook(&session_lifecycle_picker_scope(), hook);
}

// ── Actions ─────────────────────────────────────────────────────────────

/// Opens the picker from anywhere in the app.
///
/// Requesting the transition is all a caller does: the picker's scope-enter
/// hook builds the rows and clears the filter, so every opener shows the same
/// fresh menu.
fn open_session_lifecycle_picker(
    _ctx: &mut ActionCtx<'_>,
    _cell: &LifecyclePickerCell,
) -> IntentResult {
    IntentResult::empty().with_scope_signal(ScopeSignal::Push(session_lifecycle_picker_scope()))
}

/// Enter: start the highlighted lifecycle.
///
/// A lifecycle whose setup command takes `$`-parameters cannot run until the
/// user supplies values, so this seeds the argument cell and hands off to that
/// popup instead of starting anything. A lifecycle with no parameters starts
/// immediately, and the setup function owns the scope transition.
fn confirm_session_lifecycle_picker(
    ctx: &mut ActionCtx<'_>,
    cell: &LifecyclePickerCell,
) -> IntentResult {
    // Snapshot what confirming needs before borrowing app state, so the cell
    // registry and the state can both be read without overlapping borrows.
    let mut selected = None;
    cell.update(|picker| selected = session_lifecycle_picker_actions::highlighted(picker));
    let Some((name, has_args)) = selected else {
        return IntentResult::empty();
    };

    if has_args {
        // Scope the state borrow so it ends before the cell registry is read:
        // `ctx` lends app state mutably and the slice registry immutably, and
        // holding both at once is an overlapping borrow.
        let config = ctx.config;
        let lifecycles = config.get_list::<SessionLifecycle>().unwrap_or_default();
        let Some(state) = app(ctx) else {
            return IntentResult::empty();
        };
        let template = {
            let Some(template) =
                session_lifecycle_picker_actions::setup_template(&lifecycles, &name)
            else {
                return IntentResult::empty();
            };
            // A result carries at most one scope signal, but the popup must
            // land *directly* on the scope beneath the picker — pop the
            // picker, then push the popup, in that order. Doing it on the
            // stack here rather than signalling keeps the two transitions
            // atomic, so nothing can observe the gap between them.
            state.frontend.scope_pop();
            state
                .frontend
                .scope_push(FocusScope::Dynamic(arg_input_scope()));
            template
        };
        let Some(popup) = ctx.slices.reader::<ArgInputState>(&arg_input_slot()) else {
            return IntentResult::empty();
        };
        popup.update(|cell| *cell = ArgInputState::new(name, template));
        return IntentResult::empty();
    }

    let config = ctx.config;
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };

    // No args - proceed directly. The setup function owns the scope
    // transition (clear overlays, push input), so this outcome carries no
    // close signal.
    jinn_domain::session_lifecycle::intent::handle_session_lifecycle_setup(
        state,
        &name,
        &[],
        None,
        config,
    )
}

/// Escape: close without starting anything.
fn cancel_session_lifecycle_picker(
    _ctx: &mut ActionCtx<'_>,
    _cell: &LifecyclePickerCell,
) -> IntentResult {
    IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(session_lifecycle_picker_scope()))
}

/// Starts a new session, as the key does from any picker.
fn new_session(ctx: &mut ActionCtx<'_>, _cell: &LifecyclePickerCell) -> IntentResult {
    let config = ctx.config;
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    jinn_domain::feat::session::intent::handle_session_new(state, config)
}

/// Ctrl-C: clear the filter, or close when it is already empty.
///
/// A filter with text is worth more than the picker, so the key edits first
/// and only leaves once there is nothing left to clear.
fn clear_filter_or_leave(_ctx: &mut ActionCtx<'_>, cell: &LifecyclePickerCell) -> IntentResult {
    let mut empty = false;
    cell.update(|picker| {
        if picker.selection.filter().is_empty() {
            empty = true;
        } else {
            picker.selection.clear_filter();
        }
    });
    if empty {
        return IntentResult::empty()
            .with_scope_signal(ScopeSignal::PopIf(session_lifecycle_picker_scope()));
    }
    IntentResult::empty()
}
