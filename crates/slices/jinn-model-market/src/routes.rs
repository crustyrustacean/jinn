//! The model-market popup's route rows and input hook.
//!
//! Every key the popup responds to is either a [`RouteRow`] this slice
//! attaches or a hook case. The split follows the dispatch order the
//! kernel guarantees — **bound route rows resolve before the input
//! hook** — and, decisively, the popup lives in an input-hook scope:
//! every printable key is owned by the hook, so *any* character key
//! would be unreachable as a row inside the popup's own scope. The
//! non-character keys are rows:
//!
//! - `<enter>`/`<esc>`/`<c-r>` and the four navigation keys bind as
//!   rows (`<up>`/`<down>` plus their `j`/`k` twins);
//! - every printable character is a hook case: with the filter focused
//!   it types into the filter; with the table focused, `f`/`i` focus
//!   the filter, `?` toggles the help card, and `h`/`l`/`s` run their
//!   sort actions. If those were rows, route dispatch would
//!   consume them before the hook and typing "mistral" into a focused
//!   filter would flip sorts instead of filtering.
//!
//! Focus lives *inside* the row actions: the rows fire wherever focus
//! is, so `<enter>` and `<esc>` branch on [`MarketUiState::
//! filter_focused`](jinn_model_market_msg::MarketUiState::filter_focused)
//! themselves — a focused filter turns ENTER into "accept the filter"
//! and ESC into "clear the filter".

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

/// The popup's keys as data: the keybind footer's content, in display
/// order.
///
/// The footer renders from this, and the composition tests assert every
/// entry here is a key the popup actually honors — a footer that
/// advertises a dead key is a bug. The arrow keys are deliberately
/// absent (`j`/`k` are the advertised twins; the arrows stay unadvertised
/// but bound), and the character keys name their hook-owned meanings:
/// the advertised *behavior* is what the popup honors for each key,
/// whichever mechanism binds it.
pub const MARKET_BINDINGS: &[(&str, &str)] = &[
    ("<enter>", "switch"),
    ("<esc>", "close/back"),
    ("k", "up"),
    ("j", "down"),
    ("<pgup>", "page up"),
    ("<pgdn>", "page dn"),
    ("<c-r>", "refresh"),
    ("h", "col \u{2190}"),
    ("l", "col \u{2192}"),
    ("s", "sort"),
    ("f", "filter"),
    ("?", "help"),
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
/// opens. The four sort/mix actions are deliberately *not* rows — they
/// are hook cases, so a focused filter can own their letters.
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

    attach_navigation_rows(routes, ui, data);
}

/// Attaches the six navigation rows: arrows and their `j`/`k` twins,
/// plus the two page keys.
///
/// None are `StaticIntent` rows: composition's `static_intent` table
/// knows only six route ids, none of them popup intents, and a row
/// naming an unknown id is silently dropped with a warning. `j`/`k`
/// are rows (not hook cases) because they are non-ambiguous under
/// either focus — but they are character keys, and the popup's scope is
/// an input-hook scope whose printable catch-all binds `j`/`k` to
/// insert-char intents first. The hook therefore re-serves them: a
/// table-focused `j`/`k` moves the selection through
/// [`edit_intent_nav`], a focused one types.
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
            action(ui, data, move |_ctx, ui, data| apply_nav(ui, data, step)),
        ));
    }
}

/// Applies one navigation step and follows it with the debounced
/// endpoint fetch when the newly highlighted row has no detail yet.
fn apply_nav(ui: &MarketUiCell, data: &MarketDataCell, step: Nav) -> IntentResult {
    let count = visible_count(ui, data);
    match step {
        Nav::Up => ui.update(ui_state::move_up),
        Nav::Down => ui.update(|s| ui_state::move_down(s, count)),
        Nav::PageUp => ui.update(ui_state::page_up),
        Nav::PageDown => ui.update(|s| ui_state::page_down(s, count)),
    }
    fetch_following(ui, data)
}

/// The result a highlighted-row mutation publishes: the debounced
/// endpoint fetch when the newly highlighted row has no detail yet, an
/// empty result otherwise.
///
/// Every mutation that moves the highlight — the navigation rows and
/// the hook's `j`/`k`/`h`/`l`/`s`, plus the ENTER-accept and ESC-clear
/// of a focused filter — ends by attaching this, so the detail pane
/// follows whichever key moved the selection.
fn fetch_following(ui: &MarketUiCell, data: &MarketDataCell) -> IntentResult {
    match maybe_fetch_endpoints(ui, data) {
        Some(fetch) => IntentResult::new_message(fetch),
        None => IntentResult::empty(),
    }
}

/// Which navigation key was pressed.
#[derive(Debug, Clone, Copy)]
enum Nav {
    /// `<up>`/`k` — one row.
    Up,
    /// `<down>`/`j` — one row.
    Down,
    /// `<pgup>` — one viewport.
    PageUp,
    /// `<pgdn>` — one viewport.
    PageDown,
}

/// Registers the popup's input hook: the typing carve-out plus every
/// character-key action.
///
/// The composition keymap turns this registration into the scope's
/// printable-character catch-all plus the editing keys — and because
/// the keymap's bindings beat hook consults, `j`/`k` arrive here as
/// insert-char intents and the hook serves navigation for them (they
/// cannot be rows: a row would never win the binding race against the
/// catch-all, and removing the catch-all for two keys is not possible
/// per-scope). The hook:
///
/// - with the filter focused: types every character into the filter
///   (and serves the editing intents);
/// - with the table focused: `f`/`i` focus the filter, `?` toggles the
///   help card, `j`/`k` move the selection, `h`/`l` cycle the sort
///   column, `s` flips the sort direction;
/// - returns `None` otherwise — the key does nothing.
pub fn register_market_input_hook(routes: &KeyRoutes, ui: &MarketUiCell, data: &MarketDataCell) {
    let owned_ui = ui.clone();
    let owned_data = data.clone();
    let hook: InputHook = Arc::new(move |intent: &EditIntent| {
        let focus = owned_ui.read().filter_focused;
        // Every character key classifies through one place, so the
        // "rows steal filter typing" trap is structurally impossible:
        // no character key is a route row.
        if let EditIntent::InsertChar(ch) = intent {
            return match table_char_action(&owned_ui, &owned_data, *ch) {
                CharAction::Acted(result) => Some(result),
                CharAction::Type => {
                    owned_ui.update(|s| {
                        s.filter.push(*ch);
                        s.selection = 0;
                    });
                    Some(IntentResult::empty())
                }
                CharAction::Unclaimed => None,
            };
        }
        match intent {
            EditIntent::Paste(text) if focus => {
                // Pasted text lands in the filter at its end (the filter
                // is a plain string, not a cursor buffer — the same
                // shape the insert/delete cases edit). Table-focused,
                // the arm does not match and the hook declines, so the
                // paste falls to the next surface in the routing chain.
                owned_ui.update(|s| {
                    s.filter.push_str(text);
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

/// What the hook should do with a character.
#[derive(Debug)]
enum CharAction {
    /// A table-mode action fired; the keystroke is consumed. The payload
    /// is what the action publishes — a table-focused action that moved
    /// the highlight carries the row's endpoint fetch, typing carries
    /// none.
    Acted(IntentResult),
    /// The filter is focused: the character is typing.
    Type,
    /// Nobody wants this character; let the intent fall through.
    Unclaimed,
}

/// Classifies one character and runs its table-mode action.
///
/// Table-focused `h`/`l`/`s` mutate the sort cell exactly as their
/// pre-hook-route ancestors did; `j`/`k` navigate because the
/// keymap's insert-char catch-all owns those keys in this scope. Every
/// arm that moves the highlighted row returns the row's endpoint fetch
/// through [`CharAction::Acted`] — the same follow-along the
/// navigation rows publish — so the pane tracks the vim keys too.
fn table_char_action(ui: &MarketUiCell, data: &MarketDataCell, ch: char) -> CharAction {
    if ui.read().filter_focused {
        return CharAction::Type;
    }
    match ch {
        'f' | 'i' => {
            ui.update(|s| s.filter_focused = true);
            CharAction::Acted(IntentResult::empty())
        }
        '?' => {
            ui.update(|s| s.help_visible = !s.help_visible);
            CharAction::Acted(IntentResult::empty())
        }
        'k' => {
            ui.update(ui_state::move_up);
            CharAction::Acted(fetch_following(ui, data))
        }
        'j' => {
            // Clamp against the live visible count, not the measured
            // viewport: a filter can shrink the list below the window,
            // and a highlight past the list end simply vanishes.
            let count = visible_count(ui, data);
            ui.update(|s| ui_state::move_down(s, count));
            CharAction::Acted(fetch_following(ui, data))
        }
        'h' => {
            ui.update(|s| {
                s.sort_col = s.sort_col.prev();
            });
            CharAction::Acted(fetch_following(ui, data))
        }
        'l' => {
            ui.update(|s| {
                s.sort_col = s.sort_col.next();
            });
            CharAction::Acted(fetch_following(ui, data))
        }
        's' => {
            ui.update(|s| s.sort_desc = !s.sort_desc);
            CharAction::Acted(fetch_following(ui, data))
        }
        _ => CharAction::Unclaimed,
    }
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

/// `<esc>`: a focused filter clears — text emptied, selection reset,
/// focus back on the table; the popup stays open. Clearing changes the
/// visible set, so the newly highlighted row 0's endpoint fetch follows.
/// Table-focused, the popup closes.
fn escape_market(
    _ctx: &mut ActionCtx<'_>,
    ui: &MarketUiCell,
    data: &MarketDataCell,
) -> IntentResult {
    if ui.read().filter_focused {
        ui.update(|s| {
            s.filter.clear();
            s.filter_focused = false;
            s.selection = 0;
        });
        return fetch_following(ui, data);
    }
    IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(market_ui_scope()))
}

/// ENTER: a focused filter accepts — focus returns to the table with
/// the typed filter still applied and the popup still open; accepting
/// narrows the visible set, so the filtered set's row 0 fetch follows.
/// Table focused, the confirm triple publishes — `ProviderSwitch` for
/// the highlighted row, `UpdateAppState::SetLastModel` to seed the
/// global default, and the scope pop. No highlighted row, no-op.
fn confirm_market(
    ctx: &mut ActionCtx<'_>,
    ui: &MarketUiCell,
    data: &MarketDataCell,
) -> IntentResult {
    if ui.read().filter_focused {
        ui.update(|s| s.filter_focused = false);
        return fetch_following(ui, data);
    }
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

/// Builds the `FetchEndpoints` request for the highlighted row when its
/// detail is absent, stamping the UI cell's sequence for the actor's
/// debounce.
///
/// Every highlight-moving action routes through [`fetch_following`]: a
/// row whose endpoints are not cached yet gets a debounced fetch, and a
/// stale sequence loses to a newer one in the actor. The request rides
/// the action's `RouteResult` — the same publish path as every routed
/// message, hook or row.
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
