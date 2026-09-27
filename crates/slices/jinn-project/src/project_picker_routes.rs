//! The project picker's route rows and input hook.
//!
//! Every key the picker responds to is a [`RouteRow`] this slice attaches. The
//! kernel contributes no keybind, no scope variant, and no picker identifier.
//!
//! **The `<c-enter>` chain.** This picker's most interesting key opens a
//! *second* picker — the session-lifecycle menu — from inside itself. The
//! dependency runs one way, project → session-lifecycle, so the two slices
//! never name each other's menus: the project slice imports only the
//! lifecycle *scope id* from its msg crate, exactly as it imports its own
//! entry vocabulary. The lifecycle slice knows nothing about the project
//! picker; it simply populates its own rows when its scope is pushed.

use std::sync::Arc;

use jinn_domain::state::frontend_state::PendingSessionCreation;
use jinn_preferences_config::schemas::ProjectConfig;
use jinn_project_msg::ProjectPickerState;
use jinn_slices::KeyRoutes;
use jinn_slices::RouteId;
use jinn_slices::RouteResult as IntentResult;
use jinn_slices::cell::TypedCell;
use jinn_slices::route::{
    ActionCtx, ActionFn, BindSite, EditIntent, InputHook, RouteOutcome, RouteRow, ScopeSignal,
};

use crate::project_picker_actions;
use jinn_project_msg::project_picker_scope;

/// The picker's cell — the single home for everything it shows.
type ProjectPickerCell = TypedCell<ProjectPickerState>;

/// The picker's rows as data: the keys it binds, in footer order.
///
/// The footer renders from this, and a test asserts every entry here is a key
/// the picker actually binds — a footer that advertises a dead key is a bug.
pub const PROJECT_PICKER_BINDINGS: &[(&str, &str)] = &[
    ("<c-enter>", "new+lifecycle"),
    ("<c-d>", "remove"),
    ("<enter>", "start"),
    ("<esc>", "cancel"),
];

/// Every key the picker binds in its own scope, for the wiring test.
#[must_use]
pub fn bound_keys() -> Vec<&'static str> {
    PROJECT_PICKER_BINDINGS
        .iter()
        .map(|(key, _)| *key)
        .collect()
}

/// The kernel's application state behind an [`ActionCtx`].
fn app<'a>(ctx: &'a mut ActionCtx<'_>) -> Option<&'a mut jinn_domain::AppState> {
    ctx.state
        .as_any_mut()?
        .downcast_mut::<jinn_domain::AppState>()
}

/// Wraps a picker action in an [`ActionFn`], handing it both the dispatch
/// context and the cell.
fn action<F>(cell: &ProjectPickerCell, f: F) -> ActionFn
where
    F: Fn(&mut ActionCtx<'_>, &ProjectPickerCell) -> IntentResult + Send + Sync + 'static,
{
    let cell = cell.clone();
    ActionFn::new(move |mut ctx| f(&mut ctx, &cell))
}

/// Builds one `Action` row binding `key` in the picker's own scope.
fn row(
    action_name: &'static str,
    key: &'static str,
    category: &'static str,
    display: &'static str,
    run: ActionFn,
) -> RouteRow {
    RouteRow {
        route_id: RouteId::new(action_name),
        scope: project_picker_scope(),
        key,
        category,
        site: BindSite::OwnScope,
        feature: "project",
        outcome: RouteOutcome::Action {
            action: action_name,
            display,
            run,
        },
    }
}

/// Attaches every row the project picker owns.
///
/// The opener binds in the `Normal` static scope rather than the picker's own:
/// a key that opens the picker cannot live inside the scope it opens.
pub fn attach_project_picker_rows(routes: &KeyRoutes, cell: &ProjectPickerCell) {
    routes.attach(RouteRow {
        route_id: RouteId::new("project:open"),
        scope: project_picker_scope(),
        key: "<leader>sp",
        category: "general",
        site: BindSite::StaticScopes(&["Normal"]),
        feature: "project",
        outcome: RouteOutcome::Action {
            action: "open-project-picker",
            display: "search projects",
            run: action(cell, open_project_picker),
        },
    });

    routes.attach(row(
        "new-session-with-lifecycle",
        "<c-enter>",
        "general",
        "start a session at this project through the lifecycle picker",
        action(cell, new_session_with_lifecycle),
    ));
    routes.attach(row(
        "remove-highlighted-project",
        "<c-d>",
        "input",
        "remove the highlighted project",
        action(cell, remove_highlighted_project),
    ));
    routes.attach(row(
        "confirm-project-picker",
        "<enter>",
        "general",
        "start a session at this project",
        action(cell, confirm_project_picker),
    ));
    routes.attach(row(
        "cancel-project-picker",
        "<esc>",
        "general",
        "close without starting a session",
        action(cell, cancel_project_picker),
    ));
    routes.attach(row(
        "new-session-from-project-picker",
        "<c-n>",
        "general",
        "start a new session",
        action(cell, |ctx, _cell| {
            let config = ctx.config;
            let Some(state) = app(ctx) else {
                return IntentResult::empty();
            };
            jinn_domain::feat::session::intent::handle_session_new(state, config)
        }),
    ));
    routes.attach(row(
        "clear-filter-or-leave-project-picker",
        "<c-c>",
        "general",
        "clear the filter, or close when already empty",
        action(cell, clear_filter_or_leave),
    ));

    attach_navigation_rows(routes, cell);
}

/// Attaches the list-navigation rows.
///
/// They cannot be `StaticIntent` rows: composition's `static_intent` table
/// knows only six route ids, none of them picker intents, and a row naming an
/// unknown id is silently dropped with a warning.
fn attach_navigation_rows(routes: &KeyRoutes, cell: &ProjectPickerCell) {
    for (name, key, display, step) in [
        ("move-project-picker-up", "<up>", "move up", Nav::Up),
        ("move-project-picker-down", "<down>", "move down", Nav::Down),
        ("page-project-picker-up", "<pgup>", "page up", Nav::PageUp),
        (
            "page-project-picker-down",
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
pub fn register_project_picker_input_hook(routes: &KeyRoutes, cell: &ProjectPickerCell) {
    let owned = cell.clone();
    let hook: InputHook = Arc::new(move |intent: &EditIntent| {
        owned.update(|picker| match intent {
            EditIntent::InsertChar(ch) => picker.selection.insert_char(*ch),
            EditIntent::DeleteBackward | EditIntent::DeleteForward => {
                picker.selection.backspace();
            }
            EditIntent::CursorLeft => picker.selection.move_cursor_left(),
            EditIntent::CursorRight => picker.selection.move_cursor_right(),
            EditIntent::CursorHome | EditIntent::CursorEnd => {}
            EditIntent::Paste(text) => picker.selection.insert_text(text),
        });
        Some(IntentResult::empty())
    });
    routes.register_input_hook(&project_picker_scope(), hook);
}

/// Opens the picker: a fresh filter and one row per curated project.
fn open_project_picker(ctx: &mut ActionCtx<'_>, cell: &ProjectPickerCell) -> IntentResult {
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    let theme = state.frontend.theme.clone();
    let projects = ctx.config.get_list::<ProjectConfig>().unwrap_or_default();
    cell.update(|picker| project_picker_actions::open(picker, &projects, &theme));
    IntentResult::empty().with_scope_signal(ScopeSignal::Push(project_picker_scope()))
}

/// `<c-enter>`: stash the chosen directory, leave this picker, and chain into
/// the session-lifecycle picker, which populates its own rows on push.
fn new_session_with_lifecycle(ctx: &mut ActionCtx<'_>, cell: &ProjectPickerCell) -> IntentResult {
    let Some(path) = selected_path(cell) else {
        return IntentResult::empty();
    };
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    state.frontend.pending_creation = Some(PendingSessionCreation {
        project_dir: path.clone(),
        starting_cwd: path,
    });
    // A slice-owned push replaces this picker's scope, so the stack grows by
    // one rather than two and Escape from the lifecycle menu returns here.
    IntentResult::empty().with_scope_signal(ScopeSignal::Push(
        jinn_session_lifecycle_msg::session_lifecycle_picker_scope(),
    ))
}

/// `<c-d>`: remove the highlighted project. The picker stays open, and the
/// removal is persisted so the curated list survives a restart.
fn remove_highlighted_project(ctx: &mut ActionCtx<'_>, cell: &ProjectPickerCell) -> IntentResult {
    let mut removed = None;
    cell.update(|picker| removed = project_picker_actions::remove_highlighted(picker));
    let Some(path) = removed else {
        return IntentResult::empty();
    };
    remove_project(ctx.config, &path);
    IntentResult::empty()
}

/// Drops `path` from the curated list and writes the document back.
///
/// A failed write is logged, not surfaced, for the reason the project-add
/// input gives: the row is already gone from the open picker, and refusing
/// to act would strand the user.
fn remove_project(config: &jinn_preferences_config::ConfigLayer, path: &std::path::PathBuf) {
    let mut projects = config.get_list::<ProjectConfig>().unwrap_or_default();
    let before = projects.len();
    projects.retain(|project| project.path != *path);
    if projects.len() == before {
        return;
    }
    if let Err(error) = config.put_list::<ProjectConfig>(&projects) {
        tracing::warn!(err = ?error, "failed to persist the removed project to jinn.toml");
    }
}

/// `<enter>`: stash the chosen directory and run the blank lifecycle setup,
/// which owns the scope transition to `Normal`.
fn confirm_project_picker(ctx: &mut ActionCtx<'_>, cell: &ProjectPickerCell) -> IntentResult {
    let Some(path) = selected_path(cell) else {
        return IntentResult::empty();
    };
    // Read the layer before the state borrow: `app` takes `ctx` mutably.
    let config = ctx.config;
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    state.frontend.pending_creation = Some(PendingSessionCreation {
        project_dir: path.clone(),
        starting_cwd: path,
    });
    state.frontend.scope_pop();
    let result = jinn_domain::feat::session_lifecycle::intent::handle_session_lifecycle_setup(
        state,
        "",
        &[],
        None,
        config,
    );
    jinn_domain::common::slices::key_routes::into_route_result(result)
}

/// `<esc>`: leave without starting a session.
fn cancel_project_picker(_ctx: &mut ActionCtx<'_>, _cell: &ProjectPickerCell) -> IntentResult {
    IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(project_picker_scope()))
}

/// `<c-c>`: clear the filter, or leave when it is already empty.
fn clear_filter_or_leave(_ctx: &mut ActionCtx<'_>, cell: &ProjectPickerCell) -> IntentResult {
    let mut filter_empty = false;
    cell.update(|picker| {
        if picker.selection.filter().is_empty() {
            filter_empty = true;
        } else {
            picker.selection.clear_filter();
        }
    });
    if filter_empty {
        return IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(project_picker_scope()));
    }
    IntentResult::empty()
}

/// The highlighted project's path, if a row is highlighted.
fn selected_path(cell: &ProjectPickerCell) -> Option<std::path::PathBuf> {
    let guard = cell.read();
    project_picker_actions::selected_path(&guard)
}
