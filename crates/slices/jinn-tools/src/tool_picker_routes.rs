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

//! The tool picker's route rows and input hook.
//!
//! Every key the tool picker responds to is a [`RouteRow`] this slice attaches
//! itself; the kernel contributes no keybind, no scope variant, and no picker
//! identifier.
//!
//! Two shapes are worth knowing:
//!
//! - **Actions** carry a closure that runs against the picker's own cell and
//!   reach session state only through [`SliceActionState::as_any_mut`] — the
//!   same seam `jinn-sidebar`, `jinn-term`, `jinn-project`,
//!   `jinn-session-lifecycle`, and `jinn-skills` already use.
//! - **The filter** is an *input hook* rather than rows, because `RouteRow` has
//!   no catch-all variant. Registering the hook makes the composition keymap
//!   synthesize the printable-character catch-all and the editing keys for
//!   this scope, so typing keeps working.
//!
//! A cell guard is never held across an await: every action snapshots what it
//! needs, mutates, and drops the guard before it returns.

use std::sync::Arc;

use jinn_slices::KeyRoutes;
use jinn_slices::RouteId;
use jinn_slices::RouteResult as IntentResult;
use jinn_slices::cell::TypedCell;
use jinn_slices::route::{
    ActionCtx, ActionFn, BindSite, EditIntent, InputHook, RouteOutcome, RouteRow, ScopeSignal,
};
use jinn_tools_msg::{ToolPickerState, tool_picker_scope};

use crate::tool_picker_actions::{self, ToolRow};

/// The picker's cell — the single home for everything it shows.
type ToolPickerCell = TypedCell<ToolPickerState>;

/// The picker's rows as data: the keys it binds, in footer order.
///
/// The footer renders from this, and a test asserts every entry here is a key
/// the picker actually binds — a footer that advertises a dead key is a bug.
pub const TOOL_PICKER_BINDINGS: &[(&str, &str)] = &[
    ("<tab>", "toggle"),
    ("<enter>", "apply"),
    ("<esc>", "cancel"),
];

/// The kernel's application state behind an [`ActionCtx`].
///
/// The open, confirm, and escape actions must change session state, so they
/// downcast. This is the established slice-side seam; when the state is not the
/// kernel's (a test double), the caller gets `None` and the action declines
/// rather than panicking.
fn app<'a>(ctx: &'a mut ActionCtx<'_>) -> Option<&'a mut jinn_domain::AppState> {
    ctx.state
        .as_any_mut()?
        .downcast_mut::<jinn_domain::AppState>()
}

/// Wraps a picker action in an [`ActionFn`], handing it both the dispatch
/// context and the cell.
fn action<F>(cell: &ToolPickerCell, f: F) -> ActionFn
where
    F: Fn(&mut ActionCtx<'_>, &ToolPickerCell) -> IntentResult + Send + Sync + 'static,
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
        scope: tool_picker_scope(),
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

/// Attaches every row the tool picker owns.
///
/// The opener binds in the `Normal` static scope rather than the picker's own:
/// a key that opens the picker cannot live inside the scope it opens.
pub fn attach_tool_picker_rows(routes: &KeyRoutes, cell: &ToolPickerCell) {
    routes.attach(RouteRow {
        route_id: RouteId::new("tools:open-tool-picker"),
        scope: tool_picker_scope(),
        key: "<leader>st",
        category: "general",
        site: BindSite::StaticScopes(&["Normal"]),
        feature: "tools",
        outcome: RouteOutcome::Action {
            action: "open-tool-picker",
            display: "search tools",
            run: action(cell, open_tool_picker),
        },
    });

    routes.attach(row(
        "toggle-highlighted-tool",
        "<tab>",
        "input",
        "toggle the highlighted tool and advance",
        action(cell, toggle_highlighted_tool),
    ));
    routes.attach(row(
        "confirm-tool-picker",
        "<enter>",
        "general",
        "apply the toggled tools and close",
        action(cell, confirm_tool_picker),
    ));
    routes.attach(row(
        "cancel-tool-picker",
        "<esc>",
        "general",
        "close without changing which tools are enabled",
        action(cell, cancel_tool_picker),
    ));
    routes.attach(row(
        "new-session-from-tool-picker",
        "<c-n>",
        "general",
        "start a new session",
        action(cell, new_session),
    ));
    routes.attach(row(
        "clear-filter-or-leave-tool-picker",
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
fn attach_navigation_rows(routes: &KeyRoutes, cell: &ToolPickerCell) {
    for (name, key, display, step) in [
        ("move-tool-picker-up", "<up>", "move up", Nav::Up),
        ("move-tool-picker-down", "<down>", "move down", Nav::Down),
        ("page-tool-picker-up", "<pgup>", "page up", Nav::PageUp),
        (
            "page-tool-picker-down",
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
pub fn register_tool_picker_input_hook(routes: &KeyRoutes, cell: &ToolPickerCell) {
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
    routes.register_input_hook(&tool_picker_scope(), hook);
}

// ── Actions ─────────────────────────────────────────────────────────────

/// Opens the tool picker from anywhere in the app, by pushing its scope alone.
///
/// Exposed so any other surface can open this menu without knowing how it
/// works. The open *action* is what seeds the rows, so a caller that only
/// pushes the scope gets an empty menu until a keypress reaches the action.
#[must_use]
pub fn open_from_scope(state: &mut jinn_domain::AppState) -> IntentResult {
    state
        .frontend
        .scope_push(jinn_domain::FocusScope::Dynamic(tool_picker_scope()));
    IntentResult::empty()
}

/// Opens the picker: seed the rows from the session's registered tools,
/// snapshot the live disabled set for the escape revert, then push the
/// picker's scope.
fn open_tool_picker(ctx: &mut ActionCtx<'_>, cell: &ToolPickerCell) -> IntentResult {
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };

    let seed = seed_from_session(state);
    cell.update(|picker| {
        tool_picker_actions::open(picker, &seed.rows, &seed.disabled, &seed.theme)
    });

    state
        .frontend
        .scope_push(jinn_domain::FocusScope::Dynamic(tool_picker_scope()));
    IntentResult::empty()
}

/// The rows and the disabled set the picker seeds itself from on open.
struct Seed {
    /// The tools available for the session, in display order.
    rows: Vec<ToolRow>,
    /// The session's live disabled set, snapshotted so escape can restore it.
    disabled: std::collections::HashSet<String>,
    /// The active theme, so the rows render with the right colors.
    theme: jinn_theme::Theme,
}

/// Reads the session's tool context into the picker's seed.
///
/// Only tools the session's provider can actually run are offered, and the rows
/// come back sorted case-insensitively by name so the menu is in a stable order
/// regardless of registry hash order. An absent registry yields no rows: a
/// session with no tool context offers nothing rather than panicking.
fn seed_from_session(state: &jinn_domain::AppState) -> Seed {
    let active_session = state.active_session();
    let disabled = active_session.disabled_tools().clone();
    let provider_name = active_session.model_selection().provider_name().to_owned();
    let session_id = state.session.active_session_id().clone();
    let theme = state.frontend.theme.clone();

    let mut rows: Vec<ToolRow> = state
        .tool_registry()
        .map(|registry| {
            registry
                .read()
                .tools_for_session(&session_id)
                .into_iter()
                .filter(|def| def.available_for_provider(&provider_name))
                .map(|def| ToolRow {
                    name: def.name,
                    description: def.description,
                })
                .collect()
        })
        .unwrap_or_default();
    rows.sort_by_key(|row| row.name.to_lowercase());

    Seed {
        rows,
        disabled,
        theme,
    }
}

/// Enter: commit the toggled set as the session's disabled tools and close.
///
/// This is the *only* place the session's disabled set is written. Toggling
/// edits the cell's rows; nothing reaches the live profile until the user says
/// so.
fn confirm_tool_picker(ctx: &mut ActionCtx<'_>, cell: &ToolPickerCell) -> IntentResult {
    // `update` takes a unit-returning closure, so the committed set is parked
    // beside the cell and read back after the guard is released.
    let mut disabled = std::collections::HashSet::new();
    cell.update(|picker| disabled = tool_picker_actions::confirm(picker));
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    state.active_session_mut().set_disabled_tools(disabled);
    IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(tool_picker_scope()))
}

/// Escape: restore the snapshotted disabled set and close.
///
/// The revert path, never the confirm path. Confirm clears the snapshot, so
/// after a commit there is nothing here left to restore.
fn cancel_tool_picker(ctx: &mut ActionCtx<'_>, cell: &ToolPickerCell) -> IntentResult {
    let mut restored = None;
    cell.update(|picker| restored = tool_picker_actions::cancel(picker));
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    if let Some(disabled) = restored {
        state.active_session_mut().set_disabled_tools(disabled);
    }
    IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(tool_picker_scope()))
}

/// Tab: flip the highlighted tool's enabled flag, then advance to the next
/// row. The picker stays open — a checklist is toggled in one pass.
fn toggle_highlighted_tool(_ctx: &mut ActionCtx<'_>, cell: &ToolPickerCell) -> IntentResult {
    cell.update(tool_picker_actions::toggle_highlighted);
    IntentResult::empty()
}

/// Starts a new session, as the key does from any picker.
fn new_session(ctx: &mut ActionCtx<'_>, _cell: &ToolPickerCell) -> IntentResult {
    let config = ctx.config;
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    jinn_domain::session_lifecycle::intent::handle_session_new(state, config)
}

/// Clears a non-empty filter, or closes the picker when the filter is empty.
fn clear_filter_or_leave(ctx: &mut ActionCtx<'_>, cell: &ToolPickerCell) -> IntentResult {
    let mut leave = false;
    cell.update(|picker| {
        if picker.selection.filter().is_empty() {
            leave = true;
        } else {
            picker.selection.clear_filter();
        }
    });
    if leave {
        return cancel_tool_picker(ctx, cell);
    }
    IntentResult::empty()
}
