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

//! The MCP server inspector's route rows and input hook.
//!
//! Every key is a [`RouteRow`] this slice attaches itself; the kernel
//! contributes no keybind, no scope variant, and no picker identifier. The
//! filter is an *input hook* rather than rows, because `RouteRow` has no
//! catch-all variant — registering the hook makes composition synthesize the
//! printable-character catch-all and the editing keys.

use jinn_mcp_msg::mcp_picker_scope;
use jinn_slices::KeyRoutes;
use jinn_slices::RouteId;
use jinn_slices::RouteResult as IntentResult;
use jinn_slices::cell::TypedCell;
use jinn_slices::route::{
    ActionCtx, ActionFn, BindSite, EditIntent, InputHook, RouteOutcome, RouteRow, ScopeSignal,
};

use crate::mcp_picker_actions;

/// The inspector's cell.
type McpPickerCell = TypedCell<jinn_mcp_msg::McpPickerState>;

/// The inspector's rows as data: the keys it binds, in footer order.
///
/// The footer renders from this, and a test asserts every entry is a key the
/// inspector actually binds — a footer advertising a dead key is a bug.
pub const MCP_PICKER_BINDINGS: &[(&str, &str)] = &[
    ("<tab>", "toggle"),
    ("<c-r>", "restart"),
    ("<c-t>", "logs/tools"),
    ("<enter>", "save"),
    ("<esc>", "cancel"),
];

/// The kernel's application state behind an [`ActionCtx`].
///
/// When the state is not the kernel's (a test double), the action declines
/// rather than panicking.
fn app<'a>(ctx: &'a mut ActionCtx<'_>) -> Option<&'a mut jinn_domain::AppState> {
    ctx.state
        .as_any_mut()?
        .downcast_mut::<jinn_domain::AppState>()
}

/// Wraps an inspector action in an [`ActionFn`], handing it the cell.
fn action<F>(cell: &McpPickerCell, f: F) -> ActionFn
where
    F: Fn(&mut ActionCtx<'_>, &McpPickerCell) -> IntentResult + Send + Sync + 'static,
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
        scope: mcp_picker_scope(),
        key,
        category,
        site: BindSite::OwnScope,
        feature: "mcp-slice",
        outcome: RouteOutcome::Action {
            action: route_id,
            display,
            run,
        },
    }
}

/// Attaches every row the inspector owns.
pub fn attach_mcp_picker_rows(routes: &KeyRoutes, cell: &McpPickerCell) {
    routes.attach(RouteRow {
        route_id: RouteId::new("mcp-picker:open"),
        scope: mcp_picker_scope(),
        key: "<leader>sM",
        category: "general",
        site: BindSite::StaticScopes(&["Normal"]),
        feature: "mcp-slice",
        outcome: RouteOutcome::Action {
            action: "open-mcp-picker",
            display: "search mcp servers",
            run: action(cell, open_mcp_picker),
        },
    });

    for (name, key, category, display, step) in [
        (
            "close-mcp-picker",
            "<esc>",
            "general",
            "close the inspector",
            Nav::Close,
        ),
        (
            "confirm-mcp-picker",
            "<enter>",
            "general",
            "save the enabled set",
            Nav::Confirm,
        ),
        (
            "toggle-mcp-picker",
            "<tab>",
            "general",
            "enable or disable the server",
            Nav::Tab,
        ),
        (
            "restart-mcp-picker",
            "<c-r>",
            "general",
            "restart the server",
            Nav::Restart,
        ),
        (
            "toggle-preview-mcp-picker",
            "<c-t>",
            "general",
            "flip logs and tools",
            Nav::Preview,
        ),
        (
            "move-mcp-picker-up",
            "<up>",
            "navigation",
            "move up",
            Nav::Up,
        ),
        (
            "move-mcp-picker-down",
            "<down>",
            "navigation",
            "move down",
            Nav::Down,
        ),
        (
            "page-mcp-picker-up",
            "<pgup>",
            "navigation",
            "page up",
            Nav::PageUp,
        ),
        (
            "page-mcp-picker-down",
            "<pgdn>",
            "navigation",
            "page down",
            Nav::PageDown,
        ),
    ] {
        routes.attach(row(
            name,
            key,
            category,
            display,
            action(cell, move |ctx, cell| handle(ctx, cell, step)),
        ));
    }

    routes.attach(row(
        "clear-filter-or-leave-mcp-picker",
        "<c-c>",
        "general",
        "clear the filter, or close when already empty",
        action(cell, clear_filter_or_leave),
    ));
}

/// Which of the inspector's keys was pressed.
#[derive(Debug, Clone, Copy)]
enum Nav {
    /// `<esc>` — close and revert.
    Close,
    /// `<enter>` — close and commit.
    Confirm,
    /// `<tab>` — toggle enabled, then advance.
    Tab,
    /// `<c-r>` — restart the highlighted server.
    Restart,
    /// `<c-t>` — flip the preview pane.
    Preview,
    /// `<up>` — one row.
    Up,
    /// `<down>` — one row.
    Down,
    /// `<pgup>` — half the visible window.
    PageUp,
    /// `<pgdn>` — half the visible window.
    PageDown,
}

/// Registers the inspector's filter editing hook.
pub fn register_mcp_picker_input_hook(routes: &KeyRoutes, cell: &McpPickerCell) {
    let owned = cell.clone();
    let hook: InputHook = std::sync::Arc::new(move |intent: &EditIntent| {
        owned.update(|picker| {
            let list = mcp_picker_actions::list_mut(picker);
            match intent {
                EditIntent::InsertChar(ch) => list.insert_char(*ch),
                EditIntent::DeleteBackward | EditIntent::DeleteForward => list.backspace(),
                EditIntent::CursorLeft => list.move_cursor_left(),
                EditIntent::CursorRight => list.move_cursor_right(),
                EditIntent::CursorHome | EditIntent::CursorEnd => {}
                EditIntent::Paste(text) => list.insert_text(text),
            }
        });
        Some(IntentResult::empty())
    });
    routes.register_input_hook(&mcp_picker_scope(), hook);
}

// ── Actions ─────────────────────────────────────────────────────────────

/// Opens the inspector: reset, park the session id, snapshot the enabled
/// set for `ESC`, and load the configured servers.
fn open_mcp_picker(ctx: &mut ActionCtx<'_>, cell: &McpPickerCell) -> IntentResult {
    let (servers, enabled, theme, session_id) = {
        let Some(state) = app(ctx) else {
            return IntentResult::empty();
        };
        let servers: Vec<(String, String)> = state
            .frontend
            .preferences
            .mcp_server
            .iter()
            .map(|(name, server)| (name.clone(), server.description_for_picker()))
            .collect();
        (
            servers,
            state.active_session().enabled_mcp_servers().clone(),
            state.frontend.theme.clone(),
            state.active_session().session_id().clone(),
        )
    };

    let entries = mcp_picker_actions::build_entries(&servers, &enabled, &theme);
    cell.update(|picker| {
        mcp_picker_actions::reset(picker);
        mcp_picker_actions::set_session(picker, session_id);
        mcp_picker_actions::set_snapshot(picker, enabled);
        mcp_picker_actions::load(picker, entries);
    });

    IntentResult::empty().with_scope_signal(ScopeSignal::Push(mcp_picker_scope()))
}

/// Dispatches one of the inspector's keys.
fn handle(ctx: &mut ActionCtx<'_>, cell: &McpPickerCell, step: Nav) -> IntentResult {
    match step {
        Nav::Close => close_mcp_picker(ctx, cell),
        Nav::Confirm => confirm_mcp_picker(ctx, cell),
        Nav::Tab => {
            cell.update(mcp_picker_actions::toggle_highlighted);
            IntentResult::empty()
        }
        Nav::Preview => {
            cell.update(mcp_picker_actions::toggle_preview);
            IntentResult::empty()
        }
        Nav::Restart => restart_mcp_server(ctx, cell),
        Nav::Up | Nav::Down | Nav::PageUp | Nav::PageDown => move_highlight(cell, step),
    }
}

/// Moves the highlight by the key's step, paging by the measured viewport.
fn move_highlight(cell: &McpPickerCell, step: Nav) -> IntentResult {
    cell.update(|picker| {
        let viewport = picker.results_viewport;
        let list = mcp_picker_actions::list_mut(picker);
        match step {
            Nav::Up => list.move_up(viewport),
            Nav::Down => list.move_down(viewport),
            Nav::PageUp => list.page_up(viewport),
            Nav::PageDown => list.page_down(viewport),
            _ => {}
        }
    });
    IntentResult::empty()
}

/// Escape: close, restoring the pre-open enabled set.
///
/// The revert path only. A confirm clears the snapshot, so a later escape has
/// nothing to undo — a commit is authoritative.
fn close_mcp_picker(ctx: &mut ActionCtx<'_>, cell: &McpPickerCell) -> IntentResult {
    let mut snapshot = None;
    cell.update(|picker| snapshot = mcp_picker_actions::take_snapshot(picker));
    if let (Some(enabled), Some(state)) = (snapshot, app(ctx)) {
        state.active_session_mut().set_enabled_mcp_servers(enabled);
    }
    IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(mcp_picker_scope()))
}

/// Enter: write the enabled set to the session and tell the coordinator, so
/// it spawns newly-enabled servers and kills newly-disabled ones.
fn confirm_mcp_picker(ctx: &mut ActionCtx<'_>, cell: &McpPickerCell) -> IntentResult {
    let mut enabled = std::collections::BTreeSet::new();
    cell.update(|picker| {
        enabled = mcp_picker_actions::enabled_names(picker);
        mcp_picker_actions::clear_snapshot(picker);
    });
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    let session_id = state.active_session().session_id().clone();
    state
        .active_session_mut()
        .set_enabled_mcp_servers(enabled.clone());

    IntentResult::new_message(jinn_mcp_msg::McpEnablementChanged {
        session_id,
        enabled,
    })
    .with_scope_signal(ScopeSignal::PopIf(mcp_picker_scope()))
}

/// Ctrl-R: ask the coordinator to kill and respawn the highlighted server.
///
/// Does not close the inspector — it stays open so the user can watch the
/// status cycle back to running.
fn restart_mcp_server(ctx: &mut ActionCtx<'_>, cell: &McpPickerCell) -> IntentResult {
    let mut server = None;
    cell.update(|picker| server = mcp_picker_actions::highlighted_name(picker));
    let (Some(server), Some(state)) = (server, app(ctx)) else {
        return IntentResult::empty();
    };
    let session_id = state.active_session().session_id().clone();

    IntentResult::new_message(jinn_mcp_msg::RestartMcpServer {
        session_id: session_id.clone(),
        server,
    })
    .with_message(jinn_domain::PushChatEntry {
        session_id,
        entry: jinn_domain::ChatEntry::transient("Restarting MCP server"),
    })
}

/// Ctrl-C: clear the filter, or close when it is already empty.
fn clear_filter_or_leave(_ctx: &mut ActionCtx<'_>, cell: &McpPickerCell) -> IntentResult {
    let mut empty = false;
    cell.update(|picker| {
        let list = mcp_picker_actions::list_mut(picker);
        if list.filter().is_empty() {
            empty = true;
        } else {
            while !list.filter().is_empty() {
                list.backspace();
            }
        }
    });
    if empty {
        return IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(mcp_picker_scope()));
    }
    IntentResult::empty()
}
