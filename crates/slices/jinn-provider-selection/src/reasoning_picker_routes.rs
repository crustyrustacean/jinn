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

//! The reasoning picker's route rows and input hook.
//!
//! Every key the reasoning picker responds to is a [`RouteRow`] this slice
//! attaches itself; the kernel contributes no keybind, no scope variant, and
//! no picker identifier. The structure mirrors the persona and theme pickers',
//! which is what makes adding another picker a folder-local change.
//!
//! Two shapes are worth knowing:
//!
//! - **Actions** carry a closure that runs against the picker's own cell and
//!   reach session state only through [`SliceActionState::as_any_mut`] — the
//!   same seam `jinn-sidebar`, `jinn-term`, `jinn-project`, and
//!   `jinn-session-lifecycle` already use.
//! - **The filter** is an *input hook* rather than rows, because `RouteRow` has
//!   no catch-all variant. Registering the hook makes the composition keymap
//!   synthesize the printable-character catch-all and the editing keys for
//!   this scope, so typing keeps working.
//!
//! A cell guard is never held across an await: every action snapshots what it
//! needs, mutates, and drops the guard before it returns.

use std::sync::Arc;

use jinn_preferences_config::protocol::app_state_command::{AppStateUpdate, UpdateAppState};
use jinn_provider_selection_msg::{ReasoningPickerState, reasoning_picker_scope, resolve_effort};
use jinn_session_msg::MarkSessionInteracted;
use jinn_slices::KeyRoutes;
use jinn_slices::RouteId;
use jinn_slices::RouteResult as IntentResult;
use jinn_slices::cell::TypedCell;
use jinn_slices::route::{
    ActionCtx, ActionFn, BindSite, EditIntent, InputHook, RouteOutcome, RouteRow, ScopeSignal,
};

use crate::reasoning_picker_actions;

/// The picker's cell — the single home for everything it shows.
type ReasoningPickerCell = TypedCell<ReasoningPickerState>;

/// The picker's rows as data: the keys it binds, in footer order.
///
/// The footer renders from this, and a test asserts every entry is a key the
/// picker actually binds — a footer advertising a dead key is a bug.
pub const REASONING_PICKER_BINDINGS: &[(&str, &str)] = &[("<enter>", "apply"), ("<esc>", "cancel")];

/// The kernel's application state behind an [`ActionCtx`].
///
/// The open and confirm actions must change session state, so they downcast.
/// This is the established slice-side seam; when the state is not the kernel's
/// (a test double), the caller gets `None` and the action declines rather than
/// panicking.
fn app<'a>(ctx: &'a mut ActionCtx<'_>) -> Option<&'a mut jinn_domain::AppState> {
    ctx.state
        .as_any_mut()?
        .downcast_mut::<jinn_domain::AppState>()
}

/// Wraps a picker action in an [`ActionFn`], handing it both the dispatch
/// context and the cell.
fn action<F>(cell: &ReasoningPickerCell, f: F) -> ActionFn
where
    F: Fn(&mut ActionCtx<'_>, &ReasoningPickerCell) -> IntentResult + Send + Sync + 'static,
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
        scope: reasoning_picker_scope(),
        key,
        category,
        site: BindSite::OwnScope,
        feature: "provider-selection",
        outcome: RouteOutcome::Action {
            action: route_id,
            display,
            run,
        },
    }
}

/// Attaches every row the reasoning picker owns.
///
/// The opener binds in the `Normal` static scope rather than the picker's own:
/// a key that opens the picker cannot live inside the scope it opens.
pub fn attach_reasoning_picker_rows(routes: &KeyRoutes, cell: &ReasoningPickerCell) {
    routes.attach(RouteRow {
        route_id: RouteId::new("reasoning:open"),
        scope: reasoning_picker_scope(),
        key: "<leader>sr",
        category: "general",
        site: BindSite::StaticScopes(&["Normal"]),
        feature: "provider-selection",
        outcome: RouteOutcome::Action {
            action: "open-reasoning-picker",
            display: "search reasoning effort",
            run: action(cell, open_reasoning_picker),
        },
    });

    routes.attach(row(
        "confirm-reasoning-picker",
        "<enter>",
        "general",
        "apply the highlighted reasoning effort and close",
        action(cell, confirm_reasoning_picker),
    ));
    routes.attach(row(
        "cancel-reasoning-picker",
        "<esc>",
        "general",
        "close without changing the session's reasoning effort",
        action(cell, cancel_reasoning_picker),
    ));
    routes.attach(row(
        "new-session-from-reasoning-picker",
        "<c-n>",
        "general",
        "start a new session",
        action(cell, new_session),
    ));
    routes.attach(row(
        "clear-filter-or-leave-reasoning-picker",
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
fn attach_navigation_rows(routes: &KeyRoutes, cell: &ReasoningPickerCell) {
    for (name, key, display, step) in [
        ("move-reasoning-picker-up", "<up>", "move up", Nav::Up),
        (
            "move-reasoning-picker-down",
            "<down>",
            "move down",
            Nav::Down,
        ),
        ("page-reasoning-picker-up", "<pgup>", "page up", Nav::PageUp),
        (
            "page-reasoning-picker-down",
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
pub fn register_reasoning_picker_input_hook(routes: &KeyRoutes, cell: &ReasoningPickerCell) {
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
    routes.register_input_hook(&reasoning_picker_scope(), hook);
}

// ── Actions ─────────────────────────────────────────────────────────────

/// Opens the reasoning picker from anywhere in the app.
///
/// Pushing the scope is enough: the render pass fills the rows from the
/// picker's own cell, so a caller needs no picker registry and no knowledge
/// of the picker's contents.
#[must_use]
pub fn open_from_scope(state: &mut jinn_domain::AppState) -> IntentResult {
    state
        .frontend
        .scope_push(jinn_domain::FocusScope::Dynamic(reasoning_picker_scope()));
    IntentResult::empty()
}

/// Opens the picker: fresh rows over the seven effort variants, marking the
/// session's own resolved effort active, then push the picker's scope.
fn open_reasoning_picker(ctx: &mut ActionCtx<'_>, cell: &ReasoningPickerCell) -> IntentResult {
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    // The session owns its effort (seeded from the global at creation), so
    // the picker marks the session's own value — never the live global,
    // which would leak one session's choice into every override-free one.
    let active = resolve_effort(state.active_session().profile().reasoning_effort);
    let theme = state.frontend.theme.clone();

    cell.update(|picker| reasoning_picker_actions::open(picker, active, &theme));

    state
        .frontend
        .scope_push(jinn_domain::FocusScope::Dynamic(reasoning_picker_scope()));
    IntentResult::empty()
}

/// Enter: write the session's reasoning override, seed the global default,
/// and close.
///
/// The session's own value is authoritative from here — the turn dispatcher
/// reads it — so writing it is what makes the choice take effect. The global
/// seed only matters for sessions created later. The session-interacted
/// message is what persists the session immediately.
fn confirm_reasoning_picker(ctx: &mut ActionCtx<'_>, cell: &ReasoningPickerCell) -> IntentResult {
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    // Snapshot the highlighted effort out of the cell, then drop the guard
    // before writing app state.
    let effort = {
        let guard = cell.read();
        reasoning_picker_actions::highlighted_effort(&guard)
    };
    let Some(effort) = effort else {
        return IntentResult::empty();
    };

    let session_id = {
        state.active_session_mut().profile_mut().reasoning_effort = Some(effort);
        state.session.active_session_id().clone()
    };

    IntentResult::new_message(MarkSessionInteracted { session_id })
        .with_message(UpdateAppState {
            updates: vec![AppStateUpdate::SetReasoningEffort(Some(effort))],
        })
        .with_scope_signal(ScopeSignal::PopIf(reasoning_picker_scope()))
}

/// Escape: close without changing anything.
///
/// The picker keeps no snapshot — a session's effort only changes on confirm,
/// so there is no staged edit to undo.
fn cancel_reasoning_picker(_ctx: &mut ActionCtx<'_>, _cell: &ReasoningPickerCell) -> IntentResult {
    IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(reasoning_picker_scope()))
}

/// Starts a new session, as the key does from any picker.
fn new_session(ctx: &mut ActionCtx<'_>, _cell: &ReasoningPickerCell) -> IntentResult {
    let config = ctx.config;
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    jinn_domain::session_lifecycle::intent::handle_session_new(state, config)
}

/// Clears a non-empty filter, or closes the picker when the filter is empty.
fn clear_filter_or_leave(ctx: &mut ActionCtx<'_>, cell: &ReasoningPickerCell) -> IntentResult {
    let mut leave = false;
    cell.update(|picker| {
        if picker.selection.filter().is_empty() {
            leave = true;
        } else {
            picker.selection.clear_filter();
        }
    });
    if leave {
        return cancel_reasoning_picker(ctx, cell);
    }
    IntentResult::empty()
}
