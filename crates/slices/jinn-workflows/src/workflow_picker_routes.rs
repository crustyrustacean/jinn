//! The workflow picker's route rows and input hook.
//!
//! Fully slice-owned, like every picker: each key the picker responds to is
//! a [`RouteRow`] the workflows slice attaches itself; the kernel
//! contributes no keybind, no scope variant, and no picker identifier.
//!
//! Two shapes are worth knowing:
//!
//! - **Actions** carry a closure that runs against the picker's own cell,
//!   and reach app state only through [`SliceActionState::as_any_mut`] —
//!   the established seam.
//! - **The filter** is an *input hook* rather than rows, because `RouteRow`
//!   has no catch-all variant. Registering the hook makes the composition
//!   keymap synthesize the printable-character catch-all and the editing
//!   keys for this scope automatically, so typing keeps working.
//!
//! A cell guard is never held across an await: every action snapshots what
//! it needs, mutates, and drops the guard before it returns.

use std::sync::Arc;

use jinn_chat_input_msg::EnqueueUserMessage;
use jinn_core_types::ChatEntry;
use jinn_slices::KeyRoutes;
use jinn_slices::RouteId;
use jinn_slices::RouteResult as IntentResult;
use jinn_slices::cell::TypedCell;
use jinn_slices::route::{
    ActionCtx, ActionFn, BindSite, EditIntent, InputHook, RouteOutcome, RouteRow, ScopeSignal,
};
use jinn_workflows_msg::WorkflowPickerState;

use crate::workflow_picker_actions;
use jinn_workflows_msg::workflows_picker_scope as workflow_picker_scope;

/// The picker's cell — the single home for everything it shows.
type WorkflowPickerCell = TypedCell<WorkflowPickerState>;

/// The picker's rows as data: the keys it binds, in footer order.
///
/// The footer renders from this, and a test asserts every entry here is a
/// key the picker actually binds — a footer that advertises a dead key is a
/// bug.
pub const WORKFLOW_PICKER_BINDINGS: &[(&str, &str)] = &[
    ("<enter>", "start"),
    ("<c-r>", "rescan"),
    ("<c-n>", "new session"),
    ("<esc>", "cancel"),
];

/// Every key the picker binds in its own scope, for the wiring test.
#[must_use]
pub fn bound_keys() -> Vec<&'static str> {
    WORKFLOW_PICKER_BINDINGS
        .iter()
        .map(|(key, _)| *key)
        .collect()
}

/// The kernel's application state behind an [`ActionCtx`].
///
/// The picker's open and start actions must change app state, so they
/// downcast. This is the established slice-side seam, documented on
/// [`SliceActionState::as_any_mut`]; when the state is not the kernel's (a
/// test double), the caller gets `None` and the action declines rather than
/// panicking.
fn app<'a>(ctx: &'a mut ActionCtx<'_>) -> Option<&'a mut jinn_kernel::AppState> {
    ctx.state
        .as_any_mut()?
        .downcast_mut::<jinn_kernel::AppState>()
}

/// Wraps a picker action in an [`ActionFn`], handing it both the dispatch
/// context and the cell.
fn action<F>(cell: &WorkflowPickerCell, f: F) -> ActionFn
where
    F: Fn(&mut ActionCtx<'_>, &WorkflowPickerCell) -> IntentResult + Send + Sync + 'static,
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
        scope: workflow_picker_scope(),
        key,
        category,
        site: BindSite::OwnScope,
        feature: "workflows",
        outcome: RouteOutcome::Action {
            action: action_name,
            display,
            run,
        },
    }
}

/// Attaches every row the workflow picker owns.
///
/// The opener binds in the `Normal` static scope rather than the picker's
/// own: a key that opens the picker cannot live inside the scope it opens.
/// The remaining rows are `OwnScope`, so they bind only while the picker is
/// on top and cannot shadow another slice's keys.
pub fn attach_workflow_picker_rows(routes: &KeyRoutes, cell: &WorkflowPickerCell) {
    routes.attach(RouteRow {
        route_id: RouteId::new("workflows:open"),
        scope: workflow_picker_scope(),
        key: "<leader>sw",
        category: "general",
        site: BindSite::StaticScopes(&["Normal"]),
        feature: "workflows",
        outcome: RouteOutcome::Action {
            action: "open-workflow-picker",
            display: "start a workflow",
            run: action(cell, open_workflow_picker),
        },
    });

    routes.attach(row(
        "start-highlighted-workflow",
        "<enter>",
        "general",
        "start the highlighted workflow in a new session",
        action(cell, start_highlighted_workflow),
    ));
    routes.attach(row(
        "rescan-workflows",
        "<c-r>",
        "general",
        "rescan the workflows directory",
        action(cell, rescan_workflows),
    ));
    routes.attach(row(
        "new-session-from-workflow-picker",
        "<c-n>",
        "general",
        "start a new session",
        action(cell, |ctx, _cell| {
            let config = ctx.config;
            let Some(state) = app(ctx) else {
                return IntentResult::empty();
            };
            jinn_kernel::session_lifecycle::intent::handle_session_new(state, config)
        }),
    ));
    routes.attach(row(
        "cancel-workflow-picker",
        "<esc>",
        "general",
        "close without starting a workflow",
        action(cell, cancel_workflow_picker),
    ));

    attach_navigation_rows(routes, cell);
}

/// Attaches the four list-navigation rows.
///
/// They cannot be `StaticIntent` rows: composition's `static_intent` table
/// knows only shared-chrome intents, none of them picker navigation, and a
/// row naming an unknown id is silently dropped with a warning. So the
/// picker implements its own navigation, the same shape the skill picker
/// uses.
fn attach_navigation_rows(routes: &KeyRoutes, cell: &WorkflowPickerCell) {
    for (name, key, display, step) in [
        ("move-workflow-picker-up", "<up>", "move up", Nav::Up),
        (
            "move-workflow-picker-down",
            "<down>",
            "move down",
            Nav::Down,
        ),
        ("page-workflow-picker-up", "<pgup>", "page up", Nav::PageUp),
        (
            "page-workflow-picker-down",
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
            action(cell, move |ctx, cell| {
                navigate(ctx, cell, step);
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

/// Moves the highlight within the measured viewport.
fn navigate(_ctx: &mut ActionCtx<'_>, cell: &WorkflowPickerCell, nav: Nav) {
    cell.update(|picker| {
        let viewport = picker.results_viewport;
        match nav {
            Nav::Up => picker.selection.move_up(viewport),
            Nav::Down => picker.selection.move_down(viewport),
            Nav::PageUp => picker.selection.page_up(viewport),
            Nav::PageDown => picker.selection.page_down(viewport),
        }
    });
}

/// Registers the picker's filter editing hook.
///
/// The composition keymap turns this registration into the scope's
/// printable-character catch-all plus the editing keys, which is why typing
/// in the filter needs no rows of its own.
pub fn register_workflow_picker_input_hook(routes: &KeyRoutes, cell: &WorkflowPickerCell) {
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
    routes.register_input_hook(&workflow_picker_scope(), hook);
}

// ── Actions ─────────────────────────────────────────────────────────────

/// Opens the picker: seed the rows from the session's discovered workflows
/// and push the picker's scope.
///
/// The list is launch-global, but it lands on the session like every other
/// discovered resource, so the seed reads the active session's set — the
/// same place a `WorkflowsLoaded` broadcast refreshes.
fn open_workflow_picker(ctx: &mut ActionCtx<'_>, cell: &WorkflowPickerCell) -> IntentResult {
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    let discovered = state.active_session().discovered_workflows().to_vec();
    let theme = state.frontend.theme.clone();

    cell.update(|picker| {
        workflow_picker_actions::open(picker, &discovered, &theme);
    });

    IntentResult::empty().with_scope_signal(ScopeSignal::Push(workflow_picker_scope()))
}

/// `<enter>`: create a new session and enqueue the workflow's launch
/// message in it.
///
/// Session creation runs through the same [`handle_session_new`] path every
/// other session-creation caller uses — it clears overlays and pushes the
/// Input scope itself, which is why this outcome carries no additional
/// close signal (the lifecycle picker's no-args confirm path is the
/// precedent). The launch message is a plain user entry, `start <name>
/// workflow`, read by the orchestrating agent the way any prompt is.
fn start_highlighted_workflow(ctx: &mut ActionCtx<'_>, cell: &WorkflowPickerCell) -> IntentResult {
    // Capture the config first: `app(ctx)` holds a mutable borrow of the
    // context for the rest of the action (the `<c-n>` row's shape).
    let config = ctx.config;
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };

    // Snapshot the highlighted name, then drop the guard: session
    // creation mutates the same state this action borrows.
    let name = cell.update(|picker| workflow_picker_actions::highlighted_name(picker));
    let Some(name) = name else {
        return IntentResult::empty();
    };

    // A fresh session for the workflow to run in — the same blank
    // lifecycle setup `<c-n>` performs, inherited cwd and all — plus the
    // launch message enqueued into that new session.
    let created = jinn_kernel::session_lifecycle::intent::handle_session_new(state, config);
    let session_id = state.session.active_session_id().clone();
    created.with_message(EnqueueUserMessage {
        session_id,
        entry: ChatEntry::user(format!("start {name} workflow")),
    })
}

/// `<c-r>`: rescan the workflows directory.
///
/// Publishes the crossing trigger the supervisor forwards to the discovery
/// worker's workflows resource only — the skills, prompts, and context
/// scans are untouched. A transient note explains the pause.
fn rescan_workflows(ctx: &mut ActionCtx<'_>, _cell: &WorkflowPickerCell) -> IntentResult {
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    state
        .active_session_mut()
        .push_entry(ChatEntry::transient("Rescanning workflows..."));

    let session = state.active_session();
    let session_id = session.session_id().clone();
    let cwd = session.cwd().to_path_buf();

    IntentResult::new_message(jinn_workflows_msg::ScanWorkflows { session_id, cwd })
}

/// `<esc>`: close without starting anything.
fn cancel_workflow_picker(_ctx: &mut ActionCtx<'_>, _cell: &WorkflowPickerCell) -> IntentResult {
    IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(workflow_picker_scope()))
}

/// Republishes the picker's rows from a discovery scan.
///
/// Called by the slice's republisher actor when a
/// [`WorkflowsLoaded`](jinn_workflows_msg::WorkflowsLoaded) event announces
/// a new set. Without it the picker would keep showing the previous scan's
/// rows; with it the highlight stays on the workflow the user was looking
/// at rather than jumping to the top of the list.
///
/// The theme is taken from the picker's own palette, which the open action
/// already captured when the rows were built — repainting must not recolor
/// rows the user is currently looking at.
pub fn republish_from_discovery(
    cell: &WorkflowPickerCell,
    discovered: &[jinn_workflows_msg::Workflow],
) {
    cell.update(|picker| {
        let highlight = picker
            .selection
            .selected_item()
            .map(|item| item.entry().name.clone());
        let theme = picker.theme.clone();
        let items = jinn_workflows_msg::build_workflow_entries(discovered, &theme);
        picker.selection.set_items(items);
        if let Some(name) = highlight {
            restore_highlight(picker, &name);
        }
    });
}

/// Puts the highlight back on `name` after a reload, if it survived the
/// rescan.
fn restore_highlight(picker: &mut WorkflowPickerState, name: &str) {
    let Some(index) = picker
        .selection
        .items()
        .iter()
        .position(|item| item.entry().name == name)
    else {
        return;
    };
    picker.selection.set_selection(index);
}
