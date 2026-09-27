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

//! The model picker's route rows and input hook.
//!
//! Every key the picker responds to is a [`RouteRow`] this slice attaches
//! itself; the kernel contributes no keybind, no scope variant, and no picker
//! identifier.
//!
//! - **Actions** carry a closure that runs against the picker's own cell and
//!   reach app state only through [`SliceActionState::as_any_mut`].
//! - **The filter** is an *input hook* rather than rows, because `RouteRow`
//!   has no catch-all variant. Registering the hook makes the composition
//!   keymap synthesize the printable-character catch-all and editing keys.
//!
//! Alloy *mode* is read from and written to [`ProviderCell`], not the picker's
//! own state: the provider actor reads the mode while building rows, so it is
//! shared data rather than menu state.

use jinn_core_types::model_selection::ModelSelection;
use jinn_kernel::ChatEntry;
use jinn_preferences_config::protocol::app_state_command::{AppStateUpdate, UpdateAppState};
use jinn_provider_selection_msg::LoadProviderPickerEntries;
use jinn_provider_selection_msg::ProviderPickerState;
use jinn_provider_selection_msg::ProviderSwitch;
use jinn_provider_selection_msg::RefreshModels;
use jinn_slices::KeyRoutes;
use jinn_slices::RouteId;
use jinn_slices::RouteResult as IntentResult;
use jinn_slices::cell::TypedCell;
use jinn_slices::route::{
    ActionCtx, ActionFn, BindSite, EditIntent, InputHook, RouteOutcome, RouteRow, ScopeSignal,
};

use crate::provider_picker_actions;

/// The picker's cell — the single home for everything it shows.
type ProviderPickerCell = TypedCell<ProviderPickerState>;

/// The picker's rows as data: the keys it binds, in footer order.
///
/// The footer renders from this, and a test asserts every entry is a key the
/// picker actually binds — a footer advertising a dead key is a bug.
pub const PROVIDER_PICKER_BINDINGS: &[(&str, &str)] = &[
    ("<tab>", "toggle"),
    ("<c-a>", "alloy"),
    ("<c-r>", "refresh"),
    ("<enter>", "select"),
    ("<esc>", "cancel"),
];

/// The kernel's application state behind an [`ActionCtx`].
fn app<'a>(ctx: &'a mut ActionCtx<'_>) -> Option<&'a mut jinn_kernel::AppState> {
    ctx.state
        .as_any_mut()?
        .downcast_mut::<jinn_kernel::AppState>()
}

/// Wraps a picker action in an [`ActionFn`], handing it the cell.
fn action<F>(cell: &ProviderPickerCell, f: F) -> ActionFn
where
    F: Fn(&mut ActionCtx<'_>, &ProviderPickerCell) -> IntentResult + Send + Sync + 'static,
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
        scope: jinn_provider_selection_msg::provider_picker_scope(),
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

/// Attaches every row the model picker owns.
pub fn attach_provider_picker_rows(routes: &KeyRoutes, cell: &ProviderPickerCell) {
    routes.attach(RouteRow {
        route_id: RouteId::new("provider-picker:open"),
        scope: jinn_provider_selection_msg::provider_picker_scope(),
        key: "<leader>sm",
        category: "general",
        site: BindSite::StaticScopes(&["Normal"]),
        feature: "provider-selection",
        outcome: RouteOutcome::Action {
            action: "open-provider-picker",
            display: "search models",
            run: action(cell, open_provider_picker),
        },
    });

    routes.attach(row(
        "close-provider-picker",
        "<esc>",
        "general",
        "close the model browser",
        action(cell, close_provider_picker),
    ));
    routes.attach(row(
        "confirm-provider-picker",
        "<enter>",
        "general",
        "select the highlighted model",
        action(cell, confirm_provider_picker),
    ));
    routes.attach(row(
        "toggle-provider-picker",
        "<tab>",
        "general",
        "add the highlighted model to the alloy",
        action(cell, toggle_provider_picker),
    ));
    routes.attach(row(
        "toggle-provider-picker-alloy",
        "<c-a>",
        "general",
        "switch between single model and alloy",
        action(cell, toggle_alloy),
    ));
    routes.attach(row(
        "refresh-provider-picker",
        "<c-r>",
        "general",
        "refresh the model list",
        action(cell, refresh_models),
    ));
    routes.attach(row(
        "clear-filter-or-leave-provider-picker",
        "<c-c>",
        "general",
        "clear the filter, or close when already empty",
        action(cell, clear_filter_or_leave),
    ));

    attach_navigation_rows(routes, cell);
}

/// Attaches the four list-navigation rows.
///
/// Not `StaticIntent` rows: composition's `static_intent` table knows only six
/// route ids, none of them picker intents, and a row naming an unknown id is
/// silently dropped with a warning.
fn attach_navigation_rows(routes: &KeyRoutes, cell: &ProviderPickerCell) {
    for (name, key, display, step) in [
        ("move-provider-picker-up", "<up>", "move up", Nav::Up),
        (
            "move-provider-picker-down",
            "<down>",
            "move down",
            Nav::Down,
        ),
        ("page-provider-picker-up", "<pgup>", "page up", Nav::PageUp),
        (
            "page-provider-picker-down",
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
                    let list = provider_picker_actions::list_mut(picker);
                    match step {
                        Nav::Up => list.move_up(viewport),
                        Nav::Down => list.move_down(viewport),
                        Nav::PageUp => list.page_up(viewport),
                        Nav::PageDown => list.page_down(viewport),
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
pub fn register_provider_picker_input_hook(routes: &KeyRoutes, cell: &ProviderPickerCell) {
    let owned = cell.clone();
    let hook: InputHook = std::sync::Arc::new(move |intent: &EditIntent| {
        owned.update(|picker| {
            let list = provider_picker_actions::list_mut(picker);
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
    routes.register_input_hook(&jinn_provider_selection_msg::provider_picker_scope(), hook);
}

// ── Actions ─────────────────────────────────────────────────────────────

/// Opens the picker: reset, derive alloy mode from the session, and ask the
/// provider actor for the rows.
fn open_provider_picker(ctx: &mut ActionCtx<'_>, cell: &ProviderPickerCell) -> IntentResult {
    cell.update(provider_picker_actions::reset);

    // An existing Alloy opens in alloy mode, anything else in single mode.
    // Mode is cell state because the actor reads it while building rows.
    let alloy = app(ctx).is_some_and(|state| {
        matches!(
            state.active_session().profile().model,
            ModelSelection::Alloy { .. }
        )
    });
    if let Some(cell_state) = app(ctx).and_then(|state| state.provider_state()) {
        cell_state.update(|c| c.set_alloy_mode(alloy));
    }

    IntentResult::new_message(LoadProviderPickerEntries).with_scope_signal(ScopeSignal::Push(
        jinn_provider_selection_msg::provider_picker_scope(),
    ))
}

/// Escape: pop the picker. Alloy membership is *not* restored — confirm is the
/// only thing that changes the session, so abandoning the popup changes
/// nothing.
fn close_provider_picker(_ctx: &mut ActionCtx<'_>, _cell: &ProviderPickerCell) -> IntentResult {
    IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(
        jinn_provider_selection_msg::provider_picker_scope(),
    ))
}

/// Tab: flip the highlight's alloy check. Alloy mode only.
///
/// In single mode there is no set to add to, so Tab does nothing rather than
/// silently entering alloy behind the user's back.
fn toggle_provider_picker(ctx: &mut ActionCtx<'_>, cell: &ProviderPickerCell) -> IntentResult {
    // Alloy mode is read from the shared provider cell, not the picker's own.
    let Some(alloy) = alloy_mode_of(ctx) else {
        return IntentResult::empty();
    };
    if alloy {
        cell.update(provider_picker_actions::toggle_highlighted);
    }
    IntentResult::empty()
}

/// Ctrl-A: flip single/alloy. Entering pre-checks the session's current
/// models; leaving clears every check.
fn toggle_alloy(ctx: &mut ActionCtx<'_>, cell: &ProviderPickerCell) -> IntentResult {
    let Some(now_alloy) = flip_alloy_mode(ctx) else {
        return IntentResult::empty();
    };
    let Some(model_selection) =
        app(ctx).map(|state| state.active_session().profile().model.clone())
    else {
        return IntentResult::empty();
    };
    // Leaving alloy clears the checks; entering pre-checks the current models.
    let membership = if now_alloy {
        model_selection
    } else {
        ModelSelection::Single(String::new())
    };
    cell.update(|picker| provider_picker_actions::set_alloy_membership(picker, &membership));
    IntentResult::empty()
}

/// Ctrl-R: refresh the model cache through the provider actor, with a
/// transient note explaining the pause. Gated on a provider being configured.
fn refresh_models(ctx: &mut ActionCtx<'_>, _cell: &ProviderPickerCell) -> IntentResult {
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    if jinn_kernel::session_lifecycle::validator::validate_refresh_models(state).is_err() {
        return IntentResult::empty();
    }
    state
        .active_session_mut()
        .push_entry(ChatEntry::transient("Refreshing models..."));
    IntentResult::new_message(RefreshModels)
}

/// Enter: resolve single vs alloy from the checked set plus the highlight,
/// then switch the session and seed the global last-model default.
fn confirm_provider_picker(ctx: &mut ActionCtx<'_>, cell: &ProviderPickerCell) -> IntentResult {
    let Some(alloy) = alloy_mode_of(ctx) else {
        return IntentResult::empty();
    };
    let highlighted = {
        let guard = cell.read();
        provider_picker_actions::available_highlight(&guard)
    };
    let Some(highlighted) = highlighted else {
        return IntentResult::empty();
    };

    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    let model_selection = {
        let guard = cell.read();
        provider_picker_actions::resolve_selection(&guard, alloy, highlighted)
    };
    let last_model = Some(model_selection.clone());
    let session_id = state.session.active_session_id().clone();

    IntentResult::new_message(ProviderSwitch {
        session_id,
        provider_id: model_selection,
    })
    .with_message(UpdateAppState {
        updates: vec![AppStateUpdate::SetLastModel(last_model)],
    })
    .with_scope_signal(ScopeSignal::PopIf(
        jinn_provider_selection_msg::provider_picker_scope(),
    ))
}

/// Ctrl-C: clear the filter, or close when it is already empty.
fn clear_filter_or_leave(_ctx: &mut ActionCtx<'_>, cell: &ProviderPickerCell) -> IntentResult {
    let mut empty = false;
    cell.update(|picker| {
        let list = provider_picker_actions::list_mut(picker);
        if list.filter().is_empty() {
            empty = true;
        } else {
            while !list.filter().is_empty() {
                list.backspace();
            }
        }
    });
    if empty {
        return IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(
            jinn_provider_selection_msg::provider_picker_scope(),
        ));
    }
    IntentResult::empty()
}

// ── Alloy mode plumbing ─────────────────────────────────────────────────

/// Reads alloy mode out of the shared provider cell.
fn alloy_mode_of(ctx: &mut ActionCtx<'_>) -> Option<bool> {
    let cell = app(ctx)?.provider_state()?;
    Some(cell.read().is_alloy_mode())
}

/// Flips alloy mode in the shared provider cell, returning the new value.
fn flip_alloy_mode(ctx: &mut ActionCtx<'_>) -> Option<bool> {
    let cell = app(ctx)?.provider_state()?;
    let now_alloy = !cell.read().is_alloy_mode();
    cell.update(|c| c.set_alloy_mode(now_alloy));
    Some(now_alloy)
}
