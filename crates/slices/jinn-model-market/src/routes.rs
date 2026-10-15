//! The model-market popup's route rows and input hook.
//!
//! Every key the popup responds to is either a [`RouteRow`] this slice
//! attaches or part of the filter's typing carve-out. The split follows
//! the dispatch order the kernel guarantees — **route rows resolve
//! before the input hook for bound keys** — so:
//!
//! - every non-typing key (navigation, `<esc>`, `<enter>`, sort, mix,
//!   help, refresh) is a bound row;
//! - `f`/`i` and every printable character are NOT rows. The hook owns
//!   them: when the filter is focused they type into it, and when it is
//!   not, `f`/`i` focus it. If they were rows, typing `f` into a
//!   focused filter would re-trigger "focus filter" forever.
//!
//! `?` is a hook case too: the keymap's `?` binding is scoped to
//! `Normal` only, so in this dynamic scope the hook gets it and toggles
//! the help card.

use std::sync::Arc;

use jinn_core_types::model_selection::ModelSelection;
use jinn_kernel::common::app_state::AppState;
use jinn_model_market_msg::FetchEndpoints;
use jinn_model_market_msg::FetchModels;
use jinn_model_market_msg::MarketDataState;
use jinn_model_market_msg::MarketRow;
use jinn_model_market_msg::MarketUiState;
use jinn_model_market_msg::RefreshAll;
use jinn_model_market_msg::market_ui_scope;
use jinn_preferences_config::protocol::app_state_command::AppStateUpdate;
use jinn_preferences_config::protocol::app_state_command::UpdateAppState;
use jinn_provider_selection_msg::ProviderSwitch;
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
use jinn_slices::route::SliceActionState;

use crate::ui_state;

/// The UI cell — the sync side's home for selection, filter, sort, mix.
type MarketUiCell = TypedCell<MarketUiState>;
/// The data cell — the actors' table; read-only from here.
type MarketDataCell = TypedCell<MarketDataState>;

/// The count the navigation keys clamp against: the visible projection's
/// length, computed through the same filter/sort the render pass shows.
fn visible_count(ui: &MarketUiCell, data: &MarketDataCell) -> usize {
    let d = data.read();
    let guard = ui.read();
    ui_state::visible_rows(&d, &guard).len()
}

/// The highlighted row under the current filter/sort, if any.
fn highlighted(ui: &MarketUiCell, data: &MarketDataCell) -> Option<MarketRow> {
    let d = data.read();
    let guard = ui.read();
    ui_state::visible_rows(&d, &guard)
        .get(guard.selection)
        .cloned()
}

/// The popup's rows as data: the keys it binds, in footer order.
///
/// The footer renders from this, and a test asserts every entry here is
/// a key the popup actually binds — a footer that advertises a dead key
/// is a bug. `f`/`i`/typing are deliberately absent: they are the
/// hook's, not rows, and the filter bar labels them itself. `?` is the
/// hook's too — the keymap's `?` binding is Normal-scoped, so a help
/// row here would never fire; the help card names the key instead.
pub const MARKET_BINDINGS: &[(&str, &str)] = &[
    ("<enter>", "switch model"),
    ("<esc>", "table: close \u{b7} filter: back"),
    ("<up>", "up"),
    ("<down>", "down"),
    ("<pgup>", "page up"),
    ("<pgdn>", "page down"),
    ("<c-r>", "refresh all"),
    ("h", "sort col"),
    ("l", "sort col"),
    ("s", "sort dir"),
    ("m", "mix preset"),
];

/// The kernel's application state behind an [`ActionCtx`].
fn app<'a>(ctx: &'a mut ActionCtx<'_>) -> Option<&'a mut AppState> {
    ctx.state.as_any_mut()?.downcast_mut::<AppState>()
}

/// Wraps a popup action in an [`ActionFn`], handing it both cells.
fn action<F>(ui: &MarketUiCell, data: &MarketDataCell, f: F) -> ActionFn
where
    F: Fn(&mut ActionCtx<'_>, &MarketUiCell, &MarketDataCell) -> IntentResult
        + Send
        + Sync
        + 'static,
{
    let ui = ui.clone();
    let data = data.clone();
    ActionFn::new(move |mut ctx| f(&mut ctx, &ui, &data))
}

/// Builds one own-scope row bound to a slice action.
fn row(
    route_id: &'static str,
    key: &'static str,
    category: &'static str,
    display: &'static str,
    run: ActionFn,
) -> RouteRow {
    RouteRow {
        route_id: RouteId::new(route_id),
        scope: market_ui_scope(),
        key,
        category,
        site: BindSite::OwnScope,
        feature: "model-market",
        outcome: RouteOutcome::Action {
            action: route_id,
            display,
            run,
        },
    }
}

/// Attaches every row the popup owns, plus the opener.
///
/// The opener binds in the `Normal` static scope rather than the popup's
/// own: a key that opens the popup cannot live inside the scope it
/// opens.
pub fn attach_market_rows(routes: &KeyRoutes, ui: &MarketUiCell, data: &MarketDataCell) {
    routes.attach(RouteRow {
        route_id: RouteId::new("model-market:open"),
        scope: market_ui_scope(),
        key: "<leader>so",
        category: "general",
        site: BindSite::StaticScopes(&["Normal"]),
        feature: "model-market",
        outcome: RouteOutcome::Action {
            action: "open-model-market",
            display: "model market",
            run: action(ui, data, open_market),
        },
    });

    routes.attach(row(
        "confirm-model-market",
        "<enter>",
        "general",
        "switch to the highlighted model",
        action(ui, data, confirm_market),
    ));
    routes.attach(row(
        "close-model-market",
        "<esc>",
        "general",
        "back to the table, or close it",
        action(ui, data, escape_market),
    ));
    routes.attach(row(
        "refresh-model-market",
        "<c-r>",
        "general",
        "drop every cache and refetch",
        action(ui, data, refresh_market),
    ));
    routes.attach(row(
        "sort-model-market-prev-col",
        "h",
        "navigation",
        "sort by the previous column",
        action(ui, data, |_ctx, ui, _data| {
            ui.update(|s| {
                s.sort_col = s.sort_col.prev();
            });
            IntentResult::empty()
        }),
    ));
    routes.attach(row(
        "sort-model-market-next-col",
        "l",
        "navigation",
        "sort by the next column",
        action(ui, data, |_ctx, ui, _data| {
            ui.update(|s| {
                s.sort_col = s.sort_col.next();
            });
            IntentResult::empty()
        }),
    ));
    routes.attach(row(
        "sort-model-market-toggle-dir",
        "s",
        "navigation",
        "flip the sort direction",
        action(ui, data, |_ctx, ui, _data| {
            ui.update(|s| {
                s.sort_desc = !s.sort_desc;
            });
            IntentResult::empty()
        }),
    ));
    routes.attach(row(
        "cycle-model-market-mix",
        "m",
        "general",
        "next mix preset",
        action(ui, data, cycle_mix),
    ));

    attach_navigation_rows(routes, ui, data);
}

/// Attaches the four navigation rows.
///
/// Not `StaticIntent` rows: composition's `static_intent` table knows
/// only six route ids, none of them popup intents, and a row naming an
/// unknown id is silently dropped with a warning.
fn attach_navigation_rows(routes: &KeyRoutes, ui: &MarketUiCell, data: &MarketDataCell) {
    for (name, key, display, step) in [
        ("move-model-market-up", "<up>", "move up", Nav::Up),
        ("move-model-market-down", "<down>", "move down", Nav::Down),
        ("page-model-market-up", "<pgup>", "page up", Nav::PageUp),
        (
            "page-model-market-down",
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
            action(ui, data, move |_ctx, ui, data| {
                let count = visible_count(ui, data);
                match step {
                    Nav::Up => ui.update(ui_state::move_up),
                    Nav::Down => ui.update(|s| ui_state::move_down(s, count)),
                    Nav::PageUp => ui.update(ui_state::page_up),
                    Nav::PageDown => ui.update(|s| ui_state::page_down(s, count)),
                }
                let mut result = IntentResult::empty();
                if let Some(fetch) = maybe_fetch_endpoints(ui, data) {
                    result = result.with_message(fetch);
                }
                result
            }),
        ));
    }
}

/// Which navigation key was pressed.
#[derive(Debug, Clone, Copy)]
enum Nav {
    /// `<up>` — one row.
    Up,
    /// `<down>` — one row.
    Down,
    /// `<pgup>` — one viewport.
    PageUp,
    /// `<pgdn>` — one viewport.
    PageDown,
}

/// Registers the filter's typing carve-out.
///
/// The composition keymap turns this registration into the scope's
/// printable-character catch-all plus the editing keys. The hook:
///
/// - with the filter focused, types into it (and serves the editing
///   intents);
/// - with the table focused, claims `f`/`i` to focus the filter and `?`
///   to toggle the help card (the keymap's `?` is Normal-scoped, so
///   nothing else owns it here);
/// - returns `None` otherwise — the key does nothing.
pub fn register_market_input_hook(routes: &KeyRoutes, ui: &MarketUiCell, _data: &MarketDataCell) {
    let owned_ui = ui.clone();
    let hook: InputHook = Arc::new(move |intent: &EditIntent| {
        // `?` travels as an insert-char editing intent; decide by the
        // character, not by the intent kind.
        if let EditIntent::InsertChar('?') = intent
            && !owned_ui.read().filter_focused
        {
            owned_ui.update(|s| s.help_visible = !s.help_visible);
            return Some(IntentResult::empty());
        }
        let focus = owned_ui.read().filter_focused;
        match intent {
            EditIntent::InsertChar(ch) if !focus && (*ch == 'f' || *ch == 'i') => {
                owned_ui.update(|s| s.filter_focused = true);
                Some(IntentResult::empty())
            }
            EditIntent::InsertChar(ch) if focus => {
                owned_ui.update(|s| {
                    s.filter.push(*ch);
                    s.selection = 0;
                });
                Some(IntentResult::empty())
            }
            EditIntent::DeleteBackward | EditIntent::DeleteForward if focus => {
                owned_ui.update(|s| {
                    s.filter.pop();
                    s.selection = 0;
                });
                Some(IntentResult::empty())
            }
            EditIntent::CursorLeft | EditIntent::CursorRight if focus => {
                // The filter is a plain string, not a cursor buffer: the
                // cursor keys do nothing but stay claimed so they cannot
                // leak into the table while typing.
                Some(IntentResult::empty())
            }
            _ => None,
        }
    });
    routes.register_input_hook(&market_ui_scope(), hook);
}

// ── Actions ─────────────────────────────────────────────────────────────

/// Opens the popup: reset the UI cell to the opener defaults, publish
/// `FetchModels` (the actor serves warm cache or network), and push the
/// scope.
fn open_market(
    _ctx: &mut ActionCtx<'_>,
    ui: &MarketUiCell,
    _data: &MarketDataCell,
) -> IntentResult {
    ui.update(MarketUiState::reset);
    IntentResult::new_message(FetchModels).with_scope_signal(ScopeSignal::Push(market_ui_scope()))
}

/// `<esc>`: focused filter hands focus back; otherwise the popup closes.
fn escape_market(
    _ctx: &mut ActionCtx<'_>,
    ui: &MarketUiCell,
    _data: &MarketDataCell,
) -> IntentResult {
    let was_focused = ui.read().filter_focused;
    if was_focused {
        ui.update(|s| {
            s.filter_focused = false;
        });
        return IntentResult::empty();
    }
    IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(market_ui_scope()))
}

/// ENTER: publish the confirm triple — `ProviderSwitch` for the
/// highlighted row, `UpdateAppState::SetLastModel` to seed the global
/// default, and the scope pop. No highlighted row, no-op.
fn confirm_market(
    ctx: &mut ActionCtx<'_>,
    ui: &MarketUiCell,
    data: &MarketDataCell,
) -> IntentResult {
    let highlighted = highlighted(ui, data);
    let Some(row) = highlighted else {
        return IntentResult::empty();
    };
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    // The trait surface a slice action may read; `active_session_id`
    // resolves through it, not through a kernel-only method.
    let session_id = state.active_session_id();
    let last_model = Some(ModelSelection::Single(row.id.clone()));

    IntentResult::new_message(ProviderSwitch {
        session_id,
        provider_id: ModelSelection::Single(row.id),
    })
    .with_message(UpdateAppState {
        updates: vec![AppStateUpdate::SetLastModel(last_model)],
    })
    .with_scope_signal(ScopeSignal::PopIf(market_ui_scope()))
}

/// `<c-r>`: refresh everything. The publish is the whole action — the
/// actor drops the caches, refetches, and reports failures as events.
fn refresh_market(
    _ctx: &mut ActionCtx<'_>,
    _ui: &MarketUiCell,
    _data: &MarketDataCell,
) -> IntentResult {
    IntentResult::new_message(RefreshAll)
}

/// `m`: cycle the mix preset; PROJ re-sorts live because it is computed
/// at render time from this cell.
fn cycle_mix(_ctx: &mut ActionCtx<'_>, ui: &MarketUiCell, _data: &MarketDataCell) -> IntentResult {
    ui.update(MarketUiState::cycle_mix);
    IntentResult::empty()
}

/// Builds the `FetchEndpoints` request for the highlighted row when its
/// detail is absent, stamping the UI cell's sequence for the actor's
/// debounce.
///
/// `j`/`k` navigation actions call this after moving: a row whose
/// endpoints are not cached yet gets a debounced fetch, and a stale
/// sequence loses to a newer one in the actor. The request rides the
/// action's `RouteResult` — the same publish path as every routed
/// message.
fn maybe_fetch_endpoints(ui: &MarketUiCell, data: &MarketDataCell) -> Option<FetchEndpoints> {
    let request = {
        let highlighted = highlighted(ui, data);
        match highlighted {
            Some(row) if !has_detail_read(data, &row.id) => Some(row.id),
            _ => None,
        }
    };
    let model_id = request?;
    let seq = ui.update(|s| {
        s.fetch_seq += 1;
        s.fetch_seq
    });
    Some(FetchEndpoints { model_id, seq })
}

/// Whether the data cell already carries detail for `model_id`.
fn has_detail_read(data: &MarketDataCell, model_id: &str) -> bool {
    let d = data.read();
    has_detail(&d, model_id)
}

/// Whether a data snapshot already carries detail for `model_id`.
fn has_detail(data: &MarketDataState, model_id: &str) -> bool {
    data.detail.as_ref().is_some_and(|d| d.model_id == model_id)
}
