//! The attendant report-history picker's route rows and input hook.
//!
//! The picker binds in its own dynamic scope; the sidebar's `s` key in the
//! attendants section calls this slice's exported opener action (the
//! `task_list_picker_opener` precedent — a published `DynamicIntent` would
//! go to the bus and never return through route dispatch).
//!
//! The filter is an input hook rather than rows, because `RouteRow` has no
//! catch-all variant: registering the hook makes the composition keymap
//! synthesize the printable-character catch-all and the editing keys for
//! this scope. The picker is read-only, so `<enter>` is deliberately
//! unbound — there is nothing to confirm.

use jinn_attendant_msg::{attendant_report_picker_scope, attendant_report_picker_slot};
use jinn_slices::KeyRoutes;
use jinn_slices::RouteId;
use jinn_slices::RouteResult as IntentResult;
use jinn_slices::cell::TypedCell;
use jinn_slices::route::ActionCtx;
use jinn_slices::route::ActionFn;
use jinn_slices::route::BindSite;
use jinn_slices::route::EditIntent;
use jinn_slices::route::InputHook;
use jinn_slices::route::RouteOutcome;
use jinn_slices::route::RouteRow;
use jinn_slices::route::ScopeSignal;
use std::sync::Arc;

/// The picker's cell — the single home for everything it shows.
type ReportPickerCell = TypedCell<jinn_attendant_msg::AttendantReportPickerState>;

/// The picker's rows as data: the keys it binds, in footer order.
pub const REPORT_PICKER_BINDINGS: &[(&str, &str)] =
    &[("<esc>", "close"), ("<c-c>", "clear filter or close")];

/// Wraps a picker action in an [`ActionFn`], handing it the cell.
fn action<F>(cell: &ReportPickerCell, f: F) -> ActionFn
where
    F: Fn(&mut ActionCtx<'_>, &ReportPickerCell) -> IntentResult + Send + Sync + 'static,
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
        scope: attendant_report_picker_scope(),
        key,
        category,
        site: BindSite::OwnScope,
        feature: "attendant",
        outcome: RouteOutcome::Action {
            action: route_id,
            display,
            run,
        },
    }
}

/// Attaches every row the report picker owns.
pub fn attach_report_picker_rows(routes: &KeyRoutes, cell: &ReportPickerCell) {
    routes.attach(row(
        "report-picker-leave",
        "<esc>",
        "general",
        "close the report history",
        action(cell, |_ctx, cell| {
            cell.update(|picker| picker.selection.reset());
            IntentResult::empty()
                .with_scope_signal(ScopeSignal::PopIf(attendant_report_picker_scope()))
        }),
    ));
    routes.attach(row(
        "report-picker-clear-or-leave",
        "<c-c>",
        "general",
        "clear the filter, or close when already empty",
        action(cell, |_ctx, cell| {
            let was_empty = cell.update(|picker| picker.selection.filter().is_empty());
            if was_empty {
                // Already clear: `c-c` means leave.
                cell.update(|picker| picker.selection.reset());
            }
            if was_empty {
                IntentResult::empty()
                    .with_scope_signal(ScopeSignal::PopIf(attendant_report_picker_scope()))
            } else {
                IntentResult::empty()
            }
        }),
    ));

    attach_navigation_rows(routes, cell);
}

/// Attaches the four list-navigation rows.
///
/// They cannot be `StaticIntent` rows: composition's `static_intent` table
/// knows only six route ids, none of them picker intents, and a row naming
/// an unknown id is silently dropped with a warning. So the picker
/// implements its own navigation.
fn attach_navigation_rows(routes: &KeyRoutes, cell: &ReportPickerCell) {
    for (name, key, display, step) in [
        ("move-report-picker-up", "<up>", "move up", Nav::Up),
        ("move-report-picker-down", "<down>", "move down", Nav::Down),
        ("page-report-picker-up", "<pgup>", "page up", Nav::PageUp),
        (
            "page-report-picker-down",
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
/// printable-character catch-all plus the editing keys, which is why typing
/// in the filter needs no rows of its own.
pub fn register_report_picker_input_hook(routes: &KeyRoutes, cell: &ReportPickerCell) {
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
    routes.register_input_hook(&attendant_report_picker_scope(), hook);
}

/// The report-history opener, as a dispatchable action for the sidebar.
///
/// Snapshots the highlighted attendant's reports into the picker's cell and
/// pushes the picker's scope. Without the cell (slice not activated) the
/// picker simply does not open.
#[must_use]
pub fn report_picker_opener() -> ActionFn {
    ActionFn::new(move |ctx| {
        let Some(cell) = ctx.slices.reader(&attendant_report_picker_slot()) else {
            return IntentResult::empty();
        };

        // Which attendant to browse is whatever the attendants section has
        // highlighted; the opener reads it off app state so the sidebar
        // needs no handle to this slice's cell.
        let Some(state) = ctx
            .state
            .as_any_mut()
            .and_then(|s| s.downcast_mut::<jinn_kernel::common::app_state::AppState>())
        else {
            return IntentResult::empty();
        };
        let Some(reports) = crate::section_rows::highlighted_reports(state) else {
            return IntentResult::empty();
        };

        cell.update(|picker| crate::report_picker_actions::open(picker, reports));
        IntentResult::empty().with_scope_signal(ScopeSignal::Push(attendant_report_picker_scope()))
    })
}
