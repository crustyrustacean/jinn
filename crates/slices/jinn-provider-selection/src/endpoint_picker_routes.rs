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

//! The OpenRouter endpoint picker's route rows and input hook.
//!
//! Every key the picker responds to is a [`RouteRow`] this slice attaches
//! itself; the kernel contributes no keybind, no scope variant, and no picker
//! identifier.
//!
//! - **Actions** carry a closure that runs against the picker's own cell and
//!   reach app state only through [`SliceActionState::as_any_mut`].
//! - **The filter** is an *input hook* rather than rows, because `RouteRow` has
//!   no catch-all variant. Registering the hook makes the composition keymap
//!   synthesize the printable-character catch-all and the editing keys.
//!
//! The open action is also where the model-shape gate lives. It used to sit in
//! the kernel's open path — a place chosen only because the gate had to run
//! before the scope push. Now that this action *is* the push, the gate runs
//! exactly where it belongs: decide, then push, or decline and push nothing.

use std::sync::Arc;

use jinn_provider_selection_msg::LoadEndpointPickerEntries;
use jinn_provider_selection_msg::RefreshEndpointPickerEntries;
use jinn_provider_selection_msg::SetEndpointDefault;
use jinn_provider_selection_msg::endpoint::EndpointPickerState;
use jinn_provider_selection_msg::endpoint_picker_scope;
use jinn_slices::KeyRoutes;
use jinn_slices::RouteId;
use jinn_slices::RouteResult as IntentResult;
use jinn_slices::cell::TypedCell;
use jinn_slices::route::{
    ActionCtx, ActionFn, BindSite, EditIntent, InputHook, RouteOutcome, RouteRow, ScopeSignal,
};

use crate::endpoint_picker_actions;

/// The picker's cell — the single home for everything it shows.
type EndpointPickerCell = TypedCell<EndpointPickerState>;

/// The picker's rows as data: the keys it binds, in footer order.
///
/// The footer renders from this, and a test asserts every entry is a key the
/// picker actually binds — a footer advertising a dead key is a bug.
pub const ENDPOINT_PICKER_BINDINGS: &[(&str, &str)] = &[
    ("<c-r>", "refresh"),
    ("<enter>", "pin"),
    ("<esc>", "cancel"),
];

/// The kernel's application state behind an [`ActionCtx`].
///
/// Opening and confirming need the session's model shape and profile, so they
/// downcast. When the state is not the kernel's (a test double), the action
/// declines rather than panicking.
fn app<'a>(ctx: &'a mut ActionCtx<'_>) -> Option<&'a mut jinn_kernel::AppState> {
    ctx.state
        .as_any_mut()?
        .downcast_mut::<jinn_kernel::AppState>()
}

/// Wraps a picker action in an [`ActionFn`], handing it both the dispatch
/// context and the cell.
fn action<F>(cell: &EndpointPickerCell, f: F) -> ActionFn
where
    F: Fn(&mut ActionCtx<'_>, &EndpointPickerCell) -> IntentResult + Send + Sync + 'static,
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
        scope: endpoint_picker_scope(),
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

/// Attaches every row the endpoint picker owns.
///
/// The opener binds in the `Normal` static scope rather than the picker's own:
/// a key that opens the picker cannot live inside the scope it opens.
pub fn attach_endpoint_picker_rows(routes: &KeyRoutes, cell: &EndpointPickerCell) {
    routes.attach(RouteRow {
        route_id: RouteId::new("endpoint:open"),
        scope: endpoint_picker_scope(),
        key: "<leader>sE",
        category: "general",
        site: BindSite::StaticScopes(&["Normal"]),
        feature: "provider-selection",
        outcome: RouteOutcome::Action {
            action: "open-endpoint-picker",
            display: "search endpoints",
            run: action(cell, open_endpoint_picker),
        },
    });

    routes.attach(row(
        "confirm-endpoint-picker",
        "<enter>",
        "general",
        "pin the highlighted endpoint on the session",
        action(cell, confirm_endpoint_picker),
    ));
    routes.attach(row(
        "cancel-endpoint-picker",
        "<esc>",
        "general",
        "close the picker",
        action(cell, cancel_endpoint_picker),
    ));
    routes.attach(row(
        "refresh-endpoints",
        "<c-r>",
        "general",
        "force a cache-bypassing refresh",
        action(cell, refresh_endpoints),
    ));
    routes.attach(row(
        "new-session-from-endpoint-picker",
        "<c-n>",
        "general",
        "start a new session",
        action(cell, new_session),
    ));
    routes.attach(row(
        "clear-filter-or-leave-endpoint-picker",
        "<c-c>",
        "general",
        "clear the filter, or close when already empty",
        action(cell, clear_filter_or_leave),
    ));

    // The detail pane scrolls independently of the list: PageUp/PageDown page
    // the rows, so the pane needs its own keys. Trunk drove both from one
    // chat log's scroll action; here the list has dedicated paging, so the
    // pane takes Ctrl+U/Ctrl+D.
    routes.attach(row(
        "endpoint-preview-up",
        "<c-u>",
        "navigation",
        "scroll detail up",
        action(cell, scroll_detail_up),
    ));
    routes.attach(row(
        "endpoint-preview-down",
        "<c-d>",
        "navigation",
        "scroll detail down",
        action(cell, scroll_detail_down),
    ));

    attach_navigation_rows(routes, cell);
}

/// Scrolls the detail pane up one line, stopping at the top.
fn scroll_detail_up(_ctx: &mut ActionCtx<'_>, cell: &EndpointPickerCell) -> IntentResult {
    cell.update(|p| p.preview_scroll = p.preview_scroll.saturating_sub(1));
    IntentResult::empty()
}

/// Scrolls the detail pane down one line.
fn scroll_detail_down(_ctx: &mut ActionCtx<'_>, cell: &EndpointPickerCell) -> IntentResult {
    cell.update(|p| p.preview_scroll = p.preview_scroll.saturating_add(1));
    IntentResult::empty()
}

/// Attaches the four list-navigation rows.
///
/// They cannot be `StaticIntent` rows: composition's `static_intent` table
/// knows only six route ids, none of them picker intents, and a row naming an
/// unknown id is silently dropped with a warning. So the picker implements its
/// own navigation.
fn attach_navigation_rows(routes: &KeyRoutes, cell: &EndpointPickerCell) {
    for (name, key, display, step) in [
        ("move-endpoint-picker-up", "<up>", "move up", Nav::Up),
        (
            "move-endpoint-picker-down",
            "<down>",
            "move down",
            Nav::Down,
        ),
        ("page-endpoint-picker-up", "<pgup>", "page up", Nav::PageUp),
        (
            "page-endpoint-picker-down",
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
pub fn register_endpoint_picker_input_hook(routes: &KeyRoutes, cell: &EndpointPickerCell) {
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
    routes.register_input_hook(&endpoint_picker_scope(), hook);
}

// ── Actions ─────────────────────────────────────────────────────────────

/// Opens the picker, unless the session's model cannot be routed.
///
/// A single (non-alloy) model pushes the scope and asks the provider actor to
/// load the upstream list behind its cache. An alloy declines: its upstream
/// routing is chosen provider-side, so pinning one here would be meaningless.
fn open_endpoint_picker(ctx: &mut ActionCtx<'_>, cell: &EndpointPickerCell) -> IntentResult {
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    if !endpoint_picker_actions::may_route_through_endpoint(&state.active_session().profile().model)
    {
        return IntentResult::empty();
    }

    let theme = state.frontend.theme.clone();
    cell.update(|picker| endpoint_picker_actions::open(picker, Vec::new(), &theme));

    // Flag the fetch in-flight this frame so the status line shows activity
    // immediately rather than sitting on the previous fetch's age.
    if let Some(provider) = state.provider_state() {
        provider.update(|c| c.endpoint_loading = true);
    }

    IntentResult::new_message(LoadEndpointPickerEntries)
        .with_scope_signal(ScopeSignal::Push(endpoint_picker_scope()))
}

/// Enter: pin the highlighted endpoint as a per-model default.
///
/// The pin is a `[[endpoint_defaults]]` row in `providers.toml`, not session
/// state, so this action cannot apply it: an [`ActionCtx`] carries only app
/// state, the slice cells, and the config layer — never `Services`, and
/// `ConfigStorage` lives there. So the action publishes a command and the
/// provider actor performs the write.
///
/// The auto-route sentinel clears the pin rather than pinning a blank tag.
fn confirm_endpoint_picker(ctx: &mut ActionCtx<'_>, cell: &EndpointPickerCell) -> IntentResult {
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    // Snapshot the highlight as an `Option<Option<Endpoint>>`: the outer
    // option is "something is highlighted", the inner is the pin that
    // highlight implies — `None` for the auto-route sentinel, which *clears*
    // the pin rather than failing to set one. Collapsing the two would make
    // the sentinel indistinguishable from an empty menu.
    let mut selected = None;
    cell.update(|picker| {
        if picker.selection.selected_item().is_some() {
            selected = Some(endpoint_picker_actions::highlighted_endpoint(picker));
        }
    });
    let Some(endpoint) = selected else {
        return IntentResult::empty();
    };

    // The row is keyed by the model the choice applies to, not by the session
    // that made it: the pin outlives this session and covers every other one
    // using the same model. Only a `Single` selection can reach here — the
    // open action gates alloys out — so there is no rotation to resolve.
    let jinn_core_types::model_selection::ModelSelection::Single(model) =
        &state.active_session().profile().model
    else {
        return IntentResult::empty();
    };
    let model = model.clone();

    // No `MarkSessionInteracted` here: the choice is no longer session state,
    // so there is nothing on the session for a persist to carry.
    IntentResult::new_message(SetEndpointDefault {
        model,
        tag: endpoint.map(|e| e.tag),
    })
    .with_scope_signal(ScopeSignal::PopIf(endpoint_picker_scope()))
}

/// Escape: close without pinning.
fn cancel_endpoint_picker(_ctx: &mut ActionCtx<'_>, _cell: &EndpointPickerCell) -> IntentResult {
    IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(endpoint_picker_scope()))
}

/// Ctrl-R: force a cache-bypassing refresh.
///
/// The rows are cleared so the menu does not keep presenting a stale list as
/// though it were current while the fetch runs.
fn refresh_endpoints(ctx: &mut ActionCtx<'_>, cell: &EndpointPickerCell) -> IntentResult {
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    if !endpoint_picker_actions::may_route_through_endpoint(&state.active_session().profile().model)
    {
        return IntentResult::empty();
    }

    if let Some(provider) = state.provider_state() {
        provider.update(|c| c.endpoint_loading = true);
    }
    cell.update(endpoint_picker_actions::clear);
    IntentResult::new_message(RefreshEndpointPickerEntries)
}

/// Starts a new session, as the key does from any picker.
fn new_session(ctx: &mut ActionCtx<'_>, _cell: &EndpointPickerCell) -> IntentResult {
    let config = ctx.config;
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    jinn_kernel::session_lifecycle::intent::handle_session_new(state, config)
}

/// Ctrl-C: clear the filter, or close when it is already empty.
fn clear_filter_or_leave(_ctx: &mut ActionCtx<'_>, cell: &EndpointPickerCell) -> IntentResult {
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
            .with_scope_signal(ScopeSignal::PopIf(endpoint_picker_scope()));
    }
    IntentResult::empty()
}
