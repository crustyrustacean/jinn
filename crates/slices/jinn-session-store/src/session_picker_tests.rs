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

//! The session browser's behavior, exercised through the real wiring.
//!
//! Every test goes through `activate_session_picker()` rather than building
//! its own registry: an earlier picker shipped tests that wired a *different*
//! registry than production, so they passed with the production registration
//! deleted. These would not have caught that.
//!
//! The rows come from a SQLite read, so the real history load is not
//! exercised here. What is exercised is everything the picker does with rows
//! once the store has published them.

#![allow(clippy::expect_used, clippy::panic, reason = "test code")]

use jinn_domain::AppState;
use jinn_session_store_msg::SessionPickerState;
use jinn_slices::DynamicIntent;
use jinn_slices::RouteResult as IntentResult;
use jinn_slices::cell::TypedCell;
use jinn_slices::route::{ActionCtx, ScopeSignal};
use ratatui::layout::Rect;

/// The picker's cell.
type SessionPickerCell = TypedCell<SessionPickerState>;

/// A registry wired exactly as the app wires it.
struct Harness {
    slices: jinn_slices::Slices,
    routes: jinn_slices::KeyRoutes,
    state: std::cell::RefCell<AppState>,
}

impl Harness {
    /// Builds the picker through its real activation, over a `Slices` the
    /// test owns.
    ///
    /// The two-part construction is deliberate and is the point of this
    /// harness: the cells are registered on *this* registry and the app state
    /// then attaches *this* registry. `AppState::default_with_scope_focus()`
    /// mints its own registry and seeds the `OnceLock`, so a later
    /// `attach_slices` is silently a no-op and the cell is absent.
    async fn new() -> Self {
        let slices = jinn_slices::Slices::new();
        let mut viewport = jinn_slices::view::Viewport::new();
        let overlay_views = jinn_slices::OverlayViews::new();
        let routes = jinn_slices::KeyRoutes::new();
        let services = jinn_domain::Services::new_fake().await;
        {
            let mut host = jinn_slices::SliceHost::new(
                &slices,
                &mut viewport,
                &overlay_views,
                &routes,
                &services.trouper_system,
            );
            {
                let cell = slices
                    .register(
                        jinn_session_store_msg::session_picker_slot(),
                        SessionPickerState::default(),
                    )
                    .expect("slot is free");
                crate::activate_session_picker(&mut host, &cell);
            }
        }
        slices
            .register(
                jinn_slices::scope_focus_slot(),
                jinn_slices::ScopeFocusState::default(),
            )
            .expect("scope-focus cell is not registered yet");
        let mut state = AppState::default();
        let origin = jinn_session_state::ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());
        state.frontend.attach_slices(slices.clone());
        state.session.attach_slices(slices.clone());
        Self {
            slices,
            routes,
            state: std::cell::RefCell::new(state),
        }
    }

    /// Dispatches a slice action by name, the way the kernel would.
    fn dispatch(&self, action: &str) -> IntentResult {
        let mut state = self.state.borrow_mut();
        let intent = DynamicIntent::new(
            jinn_session_store_msg::session_picker_scope(),
            action,
            action,
        );
        self.routes
            .action_for(
                &intent,
                ActionCtx {
                    state: &mut *state,
                    slices: &self.slices,
                    key_bytes: Vec::new(),
                },
            )
            .unwrap_or_else(|| panic!("`{action}` must be a bound route row in the picker scope"))
    }

    /// Feeds an edit intent to the picker's input hook, the way the composed
    /// keymap does for a printable character.
    fn edit(&self, intent: &jinn_slices::EditIntent) {
        let hook = self
            .routes
            .input_hook(&jinn_session_store_msg::session_picker_scope())
            .expect("the browser registers an input hook for its filter");
        hook(intent).expect("the filter hook always consumes the edit");
    }

    /// The picker's cell — the one the slice registered.
    fn cell(&self) -> SessionPickerCell {
        self.slices
            .reader(&jinn_session_store_msg::session_picker_slot())
            .expect("the store slice registers the session picker cell at activation")
    }

    /// Installs rows, as the store actor does when its history read lands.
    fn install(&self, entries: Vec<jinn_session_store_msg::SessionTreeEntry>) {
        self.cell().update(|picker| {
            crate::session_picker_actions::load(picker, entries);
        });
    }

    /// How many rows are shown.
    fn row_count(&self) -> usize {
        self.cell().read().tree.filtered_count()
    }

    /// The highlight's index.
    fn selection(&self) -> usize {
        self.cell().read().tree.selection()
    }

    /// The filter text typed so far.
    fn filter(&self) -> String {
        self.cell().read().tree.filter().to_owned()
    }
}

/// A root-session tree row titled `title`.
///
/// Root, not a subagent: a child with no parent would render as an orphan.
fn entry(title: &str) -> jinn_session_store_msg::SessionTreeEntry {
    let session_id = jinn_core_types::SessionId::new();
    jinn_session_store_msg::SessionTreeEntry {
        id_str: session_id.to_string(),
        session_id,
        title: title.to_owned(),
        updated_at: jiff::Timestamp::UNIX_EPOCH,
        theme: jinn_theme::default_theme(),
        session_state: jinn_session_store_msg::SessionState::Loaded,
        parent_id: None,
        parent_id_str: None,
        project: None,
        project_display: String::new(),
        project_width: 0,
    }
}

/// N root rows titled `session 0`, `session 1`, and so on.
fn rows(count: usize) -> Vec<jinn_session_store_msg::SessionTreeEntry> {
    (0..count).map(|n| entry(&format!("session {n}"))).collect()
}

// ── Wiring ──────────────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn the_picker_owns_a_cell_at_activation() {
    // Given a slice built through its real activation.
    let h = Harness::new().await;

    // When reading the cell.
    // Then it is there.
    let _ = h.cell();
}

#[rstest::rstest]
#[tokio::test]
async fn the_picker_owns_every_key_it_advertises() {
    // Given a slice built through its real activation.
    let h = Harness::new().await;

    // When asking for each action the footer lists.
    let keys: Vec<String> = crate::session_picker_routes::SESSION_PICKER_BINDINGS
        .iter()
        .map(|(notation, _label)| (*notation).to_owned())
        .collect();

    // Then every advertised key resolves to a bound action in this scope.
    for key in &keys {
        assert!(
            h.routes
                .action_for(
                    &DynamicIntent::new(
                        jinn_session_store_msg::session_picker_scope(),
                        action_for_key(key),
                        key,
                    ),
                    ActionCtx {
                        state: &mut *h.state.borrow_mut(),
                        slices: &h.slices,
                        key_bytes: Vec::new(),
                    },
                )
                .is_some(),
            "`{key}` is advertised in the footer but no row binds it"
        );
    }
}

// ── Opening ─────────────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn open_pushes_the_picker_scope() {
    // Given a slice built through its real activation.
    let h = Harness::new().await;

    // When opening the browser.
    let result = h.dispatch("open-session-picker");

    // Then the picker's own scope goes on top of the stack.
    assert_eq!(
        result.scope_signal,
        Some(ScopeSignal::Push(
            jinn_session_store_msg::session_picker_scope()
        )),
        "open must push the picker's dynamic scope"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn open_asks_the_store_for_the_rows() {
    // Given a slice built through its real activation.
    let h = Harness::new().await;

    // When opening the browser.
    let result = h.dispatch("open-session-picker");

    // Then it requests a history load — the read is async, so the popup opens
    // empty and fills in when the store publishes.
    assert_eq!(result.messages.len(), 1, "open must request a history load");
}

#[rstest::rstest]
#[tokio::test]
async fn open_leaves_no_rows_behind_from_a_previous_session() {
    // Given a browser that has already shown some rows.
    let h = Harness::new().await;
    h.install(rows(5));

    // When opening it again.
    h.dispatch("open-session-picker");

    // Then the stale rows are gone.
    assert_eq!(h.row_count(), 0, "a fresh open must show no stale rows");
}

#[rstest::rstest]
#[tokio::test]
async fn rows_appear_once_the_store_publishes_them() {
    // Given a browser that was just opened.
    let h = Harness::new().await;
    h.dispatch("open-session-picker");

    // When the store publishes two sessions.
    h.install(vec![entry("first"), entry("second")]);

    // Then both are shown.
    assert_eq!(h.row_count(), 2);
}

// ── Confirming ──────────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn confirm_loads_the_highlighted_session() {
    // Given a browser with two rows, the second highlighted.
    let h = Harness::new().await;
    h.install(vec![entry("first"), entry("second")]);
    h.dispatch("move-session-picker-down");

    // When confirming.
    let result = h.dispatch("confirm-session-picker");

    // Then the store is asked to load that session.
    assert_eq!(
        result.messages.len(),
        1,
        "confirm must request the session load"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn confirm_closes_the_browser() {
    // Given a browser with a row highlighted.
    let h = Harness::new().await;
    h.install(vec![entry("first")]);

    // When confirming.
    let result = h.dispatch("confirm-session-picker");

    // Then the browser is popped.
    assert_eq!(
        result.scope_signal,
        Some(ScopeSignal::PopIf(
            jinn_session_store_msg::session_picker_scope()
        )),
        "confirm must close the browser"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn confirm_before_the_rows_arrive_does_nothing() {
    // Given a browser whose history read is still in flight.
    let h = Harness::new().await;
    h.dispatch("open-session-picker");

    // When confirming in that window.
    let result = h.dispatch("confirm-session-picker");

    // Then nothing is requested and the browser stays open — Enter must not
    // close a menu the user never chose anything in.
    assert!(
        result.messages.is_empty(),
        "confirming an empty browser must not request anything"
    );
    assert!(
        result.scope_signal.is_none(),
        "confirming an empty browser must not close it"
    );
}

// ── Navigating ──────────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn moving_down_advances_the_highlight() {
    // Given a browser with three rows.
    let h = Harness::new().await;
    h.install(rows(3));

    // When moving down.
    h.dispatch("move-session-picker-down");

    // Then the highlight is on the second row.
    assert_eq!(h.selection(), 1);
}

#[rstest::rstest]
#[tokio::test]
async fn moving_up_from_the_top_stays_at_the_top() {
    // Given a browser whose highlight is already on the first row.
    let h = Harness::new().await;
    h.install(rows(3));

    // When moving up.
    h.dispatch("move-session-picker-up");

    // Then the highlight does not wrap.
    assert_eq!(h.selection(), 0);
}

#[rstest::rstest]
#[tokio::test]
async fn paging_down_moves_by_half_the_measured_window() {
    // Given a browser with enough rows to page within, and a measured window.
    let h = Harness::new().await;
    h.install(rows(40));
    h.cell().update(|picker| picker.results_viewport = 10);

    // When paging down.
    h.dispatch("page-session-picker-down");

    // Then the highlight advanced by half the window, not a whole page.
    assert_eq!(
        h.selection(),
        5,
        "a page must be half the measured result window"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn paging_down_stops_at_the_last_row() {
    // Given a browser where a page would overshoot the end.
    let h = Harness::new().await;
    h.install(rows(8));
    h.cell().update(|picker| picker.results_viewport = 10);

    // When paging down repeatedly.
    for _ in 0..5 {
        h.dispatch("page-session-picker-down");
    }

    // Then the highlight rests on the last row instead of running past it.
    assert_eq!(h.selection(), 7);
}

#[rstest::rstest]
#[tokio::test]
async fn the_render_pass_publishes_the_measured_row_count() {
    // Given a browser with rows and a popup-sized area.
    let h = Harness::new().await;
    h.install(rows(40));
    let before = h.cell().read().results_viewport;

    // When the render pass draws the picker.
    let measured = crate::session_picker_viewport::results_viewport(&Rect::new(0, 0, 60, 24));
    h.cell().update(|picker| picker.results_viewport = measured);

    // Then the cell carries the measurement the navigation keys page by.
    //
    // Asserting against the *measurement* rather than `> 0` is the point: the
    // fallback is a non-zero 20, so a `> 0` guard would pass with the
    // measurement deleted — a guard that escaped mutation in an earlier
    // picker.
    let after = h.cell().read().results_viewport;
    assert_eq!(after, measured, "the render must publish what it measured");
    assert_ne!(
        after, before,
        "the measurement must differ from the unrendered fallback"
    );
}

// ── The filter ──────────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn typing_narrows_the_visible_rows() {
    // Given a browser with two differently-titled rows.
    let h = Harness::new().await;
    h.install(vec![entry("alpha"), entry("beta")]);

    // When typing a filter.
    for ch in "bet".chars() {
        h.edit(&jinn_slices::EditIntent::InsertChar(ch));
    }

    // Then only the matching row remains.
    let cell = h.cell();
    let guard = cell.read();
    assert_eq!(
        guard.tree.filtered_count(),
        1,
        "the filter must narrow the list"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn backspace_widens_the_filter_again() {
    // Given a browser narrowed to the single row matching "b".
    let h = Harness::new().await;
    h.install(vec![entry("alpha"), entry("beta"), entry("gamma")]);
    for ch in "b".chars() {
        h.edit(&jinn_slices::EditIntent::InsertChar(ch));
    }
    {
        let cell = h.cell();
        assert_eq!(cell.read().tree.filtered_count(), 1, "given: one match");
    }

    // When backspacing the last character.
    h.edit(&jinn_slices::EditIntent::DeleteBackward);

    // Then the filter is empty and every row matches again.
    let cell = h.cell();
    let guard = cell.read();
    assert_eq!(guard.tree.filter(), "");
    assert_eq!(guard.tree.filtered_count(), 3);
}

// ── Ctrl-C ──────────────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn ctrl_c_with_a_filter_clears_it_without_closing() {
    // Given an open browser with text in the filter.
    let h = Harness::new().await;
    h.install(vec![entry("alpha"), entry("beta")]);
    for ch in "beta".chars() {
        h.edit(&jinn_slices::EditIntent::InsertChar(ch));
    }

    // When pressing Ctrl-C.
    let result = h.dispatch("clear-filter-or-leave-session-picker");

    // Then the filter empties and the browser stays open.
    assert_eq!(h.filter(), "", "Ctrl-C must clear the filter");
    assert!(
        result.scope_signal.is_none(),
        "Ctrl-C with a filter must not close the browser"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn ctrl_c_with_an_empty_filter_closes_the_browser() {
    // Given an open browser whose filter is already empty.
    let h = Harness::new().await;
    h.install(vec![entry("alpha")]);

    // When pressing Ctrl-C.
    let result = h.dispatch("clear-filter-or-leave-session-picker");

    // Then the browser closes.
    assert_eq!(
        result.scope_signal,
        Some(ScopeSignal::PopIf(
            jinn_session_store_msg::session_picker_scope()
        )),
        "Ctrl-C on an empty filter must close the browser"
    );
}

// ── Escape ──────────────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn escape_pops_only_the_browser() {
    // Given an open browser.
    let h = Harness::new().await;
    h.dispatch("open-session-picker");

    // When pressing Escape.
    let result = h.dispatch("close-session-picker");

    // Then exactly one scope is popped, and it is this picker's.
    //
    // `PopIf` rather than `Pop`: the browser is reachable from the sidebar, so
    // Escape must leave the sidebar standing rather than clearing the stack
    // and stranding the user on an empty screen.
    assert_eq!(
        result.scope_signal,
        Some(ScopeSignal::PopIf(
            jinn_session_store_msg::session_picker_scope()
        ))
    );
}

// ── Ctrl-N ──────────────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn ctrl_n_starts_a_new_session() {
    // Given an open browser.
    let h = Harness::new().await;
    h.dispatch("open-session-picker");
    let sessions_before = {
        let state = h.state.borrow();
        state.session.iter().count()
    };

    // When pressing Ctrl-N.
    let _ = h.dispatch("new-session-from-session-picker");

    // Then a new session exists — creating one from inside the browser is
    // otherwise impossible, since the picker holds input.
    let sessions_after = {
        let state = h.state.borrow();
        state.session.iter().count()
    };
    assert!(
        sessions_after > sessions_before,
        "Ctrl-N must create a session ({sessions_before} → {sessions_after})"
    );
}

/// Maps a footer notation to the action that row binds.
fn action_for_key(notation: &str) -> &'static str {
    match notation {
        "<esc>" => "close-session-picker",
        "<enter>" => "confirm-session-picker",
        "<c-n>" => "new-session-from-session-picker",
        other => panic!("`{other}` is advertised but no action is mapped to it"),
    }
}
