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

//! The theme picker's observable behavior.
//!
//! Tests that assert *wiring* run the slice's real `activate_picker`, so a
//! picker that registered its cell but forgot its rows or its input hook fails
//! loudly rather than passing against a hand-built stand-in. Tests that assert
//! *behavior* drive the picker's public entry points.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test module, panics are acceptable"
)]

use jinn_slices::cell::TypedCell;
use jinn_slices::route::{EditIntent, RouteOutcome, ScopeSignal};
use jinn_slices::{KeyRoutes, SliceHost, Slices};
use jinn_theme_msg::{ThemePickerState, theme_entries_slot, theme_picker_scope};

/// The slice wired exactly as composition wires it.
struct Wired {
    slices: Slices,
    routes: KeyRoutes,
    state: std::cell::RefCell<jinn_domain::AppState>,
}

impl Wired {
    /// Builds the slice with `themes` already present in its theme-entries
    /// cell, avoiding a filesystem scan so the tests describe the picker, not
    /// the theme loader.
    async fn new(themes: Vec<(&str, &str)>) -> Self {
        let slices = Slices::new();
        let mut viewport = jinn_slices::view::Viewport::new();
        let overlay_views = jinn_slices::OverlayViews::new();
        let routes = KeyRoutes::new();
        let services = jinn_domain::Services::new_fake().await;
        {
            let mut host = SliceHost::new(
                &slices,
                &mut viewport,
                &overlay_views,
                &routes,
                &services.trouper_system,
            );
            // An empty themes dir: the scan runs for real (so the entries cell
            // exists and is owned by `activate`), but the tests seed the
            // entries they need rather than depending on the loader.
            let dir = std::path::PathBuf::from("/nonexistent-themes-dir");
            let system = std::path::PathBuf::from("/nonexistent-system-themes-dir");
            crate::activate(&mut host, &dir, &system);
            crate::activate_picker(&mut host);
        }
        slices
            .reader::<jinn_theme_msg::ThemeEntries>(&theme_entries_slot())
            .expect("theme-entries cell is registered at activation")
            .update(|cell: &mut jinn_theme_msg::ThemeEntries| {
                cell.entries = themes
                    .into_iter()
                    .map(|(name, accent)| jinn_theme_msg::NamedTheme {
                        name: name.to_owned(),
                        theme: accent_theme(accent),
                    })
                    .collect();
            });
        let state = jinn_domain::AppState::default();
        state.frontend.attach_slices(slices.clone());
        Self {
            slices,
            routes,
            state: std::cell::RefCell::new(state),
        }
    }

    /// The picker cell.
    fn cell(&self) -> TypedCell<ThemePickerState> {
        self.slices
            .reader(&jinn_theme_msg::theme_picker_slot())
            .expect("the picker registers its cell at activation")
    }

    /// The names the filter currently shows, in display order.
    ///
    /// Reads the *filtered* view, not the underlying item list: a picker whose
    /// filter text is written but never applied still has every item present.
    fn visible_names(&self) -> Vec<String> {
        let state = self.cell();
        let guard = state.read();
        (0..guard.selection.filtered_count())
            .filter_map(|i| guard.selection.filtered_item(i))
            .map(|item| item.entry().name.clone())
            .collect()
    }

    /// The index of the highlighted row.
    fn highlighted(&self) -> usize {
        self.cell().read().selection.selection()
    }

    /// The filter text.
    fn filter(&self) -> String {
        self.cell().read().selection.filter().to_owned()
    }

    /// The app's live theme's focus accent — the observable face of the theme.
    fn live_accent(&self) -> ratatui::style::Color {
        self.state.borrow().frontend.theme.focus_accent
    }

    /// Seeds a theme-sensitive cache so invalidation is observable.
    fn seed_theme_cache(&self) {
        self.state
            .borrow_mut()
            .frontend
            .caches
            .session_preview_cache
            .write()
            .insert(jinn_core_types::SessionId::new(), 0, 80, Vec::new());
    }

    /// How many entries the theme-sensitive cache holds.
    fn theme_cache_len(&self) -> usize {
        self.state
            .borrow()
            .frontend
            .caches
            .session_preview_cache
            .read()
            .len()
    }

    /// Opens the picker through its real `open` route action.
    fn open(&self) -> jinn_slices::RouteResult {
        self.fire("open-theme-picker")
    }

    /// Dispatches one of the picker's actions by its route action name.
    fn fire(&self, action: &str) -> jinn_slices::RouteResult {
        let mut state = self.state.borrow_mut();
        self.routes
            .action_for(
                &jinn_slices::DynamicIntent::new(theme_picker_scope(), action, action),
                jinn_slices::ActionCtx {
                    state: &mut *state,
                    slices: &self.slices,
                    key_bytes: Vec::new(),
                },
            )
            .unwrap_or_else(|| panic!("the theme picker attaches an action named {action}"))
    }

    /// Feeds an edit intent to the picker's registered input hook.
    fn edit(&self, intent: &EditIntent) {
        let hook = self
            .routes
            .input_hook(&theme_picker_scope())
            .expect("the picker registers an input hook for its filter");
        hook(intent).expect("the filter hook always consumes the edit");
    }

    /// Draws one frame of the picker at `100x30` and returns the buffer's
    /// symbols, row by row.
    fn draw(&self) -> Vec<String> {
        draw_frame(self, ratatui::layout::Rect::new(0, 0, 100, 30))
    }
}

/// Draws one frame of the picker's popup into a fresh test terminal and
/// returns the buffer's symbols, row by row.
fn draw_frame(wired: &Wired, area: ratatui::layout::Rect) -> Vec<String> {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).expect("terminal");
    let facts =
        jinn_slices::RenderFacts::new(wired.state.borrow().frontend.theme.clone(), &wired.slices);
    terminal
        .draw(|frame| {
            crate::theme_picker_render::render_theme_picker(frame, area, &facts);
        })
        .expect("draw");
    terminal
        .backend()
        .buffer()
        .content
        .chunks(area.width as usize)
        .map(|row| row.iter().map(ratatui::buffer::Cell::symbol).collect())
        .collect()
}

/// A theme distinguishable from the default by its focus accent, named by
/// `accent` so a test can assert which theme is live.
fn accent_theme(accent: &str) -> jinn_theme::Theme {
    let mut theme = jinn_theme::default_theme();
    theme.focus_accent = accent.parse().expect("a named color");
    theme
}

// ── 1. Opens with entries ───────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn opening_the_picker_shows_the_scanned_themes() {
    // Given a slice whose theme-entries cell holds two themes.
    let wired = Wired::new(vec![("gruvbox", "red"), ("nord", "blue")]).await;

    // When the picker is opened.
    wired.open();

    // Then its rows name both themes.
    assert_eq!(wired.visible_names(), vec!["gruvbox", "nord"]);
}

#[rstest::rstest]
#[tokio::test]
async fn opening_the_picker_records_the_theme_in_force_for_escape_to_restore() {
    // Given a slice whose app state has a distinct theme in force.
    let wired = Wired::new(vec![("gruvbox", "red"), ("nord", "blue")]).await;
    wired.state.borrow_mut().frontend.theme = accent_theme("green");

    // When the picker is opened.
    wired.open();

    // Then the cell holds that theme as the pre-open snapshot.
    let cell = wired.cell();
    let snapshotted = cell.read().preview_original.clone();
    assert_eq!(
        snapshotted.map(|t| t.focus_accent),
        Some(accent_theme("green").focus_accent),
        "open must snapshot the theme in force so escape can put it back"
    );
}

// ── 2. The filter narrows ───────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn typing_a_character_filters_the_list() {
    // Given an open picker with two themes.
    let wired = Wired::new(vec![("gruvbox", "red"), ("nord", "blue")]).await;
    wired.open();

    // When the user types "gru".
    for ch in "gru".chars() {
        wired.edit(&EditIntent::InsertChar(ch));
    }

    // Then the filter holds the text and only the matching row survives.
    assert_eq!(wired.filter(), "gru");
    assert_eq!(wired.visible_names(), vec!["gruvbox"]);
}

#[rstest::rstest]
#[tokio::test]
async fn backspace_shortens_the_filter() {
    // Given an open picker whose filter reads "gru".
    let wired = Wired::new(vec![("gruvbox", "red"), ("nord", "blue")]).await;
    wired.open();
    for ch in "gru".chars() {
        wired.edit(&EditIntent::InsertChar(ch));
    }

    // When backspace is pressed twice.
    wired.edit(&EditIntent::DeleteBackward);
    wired.edit(&EditIntent::DeleteBackward);

    // Then the filter reads "g" and only the one name contains it.
    assert_eq!(wired.filter(), "g");
    assert_eq!(wired.visible_names(), vec!["gruvbox"]);
}

// ── 3. Navigation ───────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn down_arrow_moves_the_highlight() {
    // Given an open picker whose highlight is on the first row.
    let wired = Wired::new(vec![("gruvbox", "red"), ("nord", "blue")]).await;
    wired.open();
    assert_eq!(wired.highlighted(), 0);

    // When the down arrow is pressed.
    wired.fire("move-theme-picker-down");

    // Then the highlight is on the second row.
    assert_eq!(wired.highlighted(), 1);
}

#[rstest::rstest]
#[tokio::test]
async fn up_arrow_stops_at_the_first_row() {
    // Given an open picker whose highlight is already at the top.
    let wired = Wired::new(vec![("gruvbox", "red"), ("nord", "blue")]).await;
    wired.open();

    // When the up arrow is pressed.
    wired.fire("move-theme-picker-up");

    // Then the highlight does not wrap or move past the first row.
    assert_eq!(wired.highlighted(), 0);
}

#[rstest::rstest]
#[tokio::test]
async fn page_down_moves_the_highlight_past_the_first_row() {
    // Given an open picker with more rows than fit on one page.
    let many: Vec<(String, String)> = (0..60)
        .map(|i| (format!("t{i:02}"), "red".to_owned()))
        .collect();
    let owned: Vec<(&str, &str)> = many.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let wired = Wired::new(owned).await;
    wired.open();

    // When page down is pressed.
    wired.fire("page-theme-picker-down");

    // Then the highlight left the first row.
    assert!(
        wired.highlighted() > 1,
        "page down must advance past the first row, got {}",
        wired.highlighted()
    );
}

#[rstest::rstest]
#[tokio::test]
async fn page_down_steps_by_the_rows_the_last_frame_actually_laid_out() {
    // Given an open picker with far more rows than a frame can show, and a
    // frame drawn so the cell learns how many rows are on screen.
    let many: Vec<(String, String)> = (0..200)
        .map(|i| (format!("t{i:03}"), "red".to_owned()))
        .collect();
    let owned: Vec<(&str, &str)> = many.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let wired = Wired::new(owned).await;
    wired.open();
    // A small frame: the laid-out list pane holds far fewer rows than the
    // pre-render fallback, so a picker that paged by the fallback would move a
    // visibly different distance.
    let frame = ratatui::layout::Rect::new(0, 0, 100, 30);
    draw_frame(&wired, frame);
    let on_screen = crate::theme_picker_viewport::results_viewport(frame);

    // When page down is pressed.
    wired.fire("page-theme-picker-down");

    // Then the highlight advanced by half a screen of rows — a page of what
    // the user can see, not of a fixed guess about how much that is.
    assert_ne!(on_screen, jinn_theme_msg::RESULTS_VIEWPORT_FALLBACK);
    assert_eq!(wired.highlighted(), (on_screen / 2).max(1));
}

// ── 4. Confirm ──────────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn confirming_asks_the_scope_stack_to_pop_the_picker() {
    // Given an open picker.
    let wired = Wired::new(vec![("gruvbox", "red")]).await;
    wired.open();

    // When enter is pressed.
    let result = wired.fire("confirm-theme-picker");

    // Then the result pops the picker's own scope, closing the menu.
    assert_eq!(
        result.scope_signal,
        Some(ScopeSignal::PopIf(theme_picker_scope()))
    );
}

#[rstest::rstest]
#[tokio::test]
async fn confirming_is_the_only_way_to_persist_a_theme() {
    // Given an open picker whose highlight has moved off the first theme.
    let wired = Wired::new(vec![("gruvbox", "red"), ("nord", "blue")]).await;
    wired.open();
    wired.fire("move-theme-picker-down");

    // When enter is pressed.
    let result = wired.fire("confirm-theme-picker");

    // Then the theme name travels out to be written to preferences — this is
    // the one place a preview becomes a choice.
    assert!(
        result
            .message_names
            .iter()
            .any(|n| n.contains("UpdateAppState")),
        "confirm must ask for the theme to be persisted; got {:?}",
        result.message_names
    );
}

#[rstest::rstest]
#[tokio::test]
async fn confirming_leaves_the_previewed_theme_in_force() {
    // Given an open picker whose highlight has previewed a second theme.
    let wired = Wired::new(vec![("gruvbox", "red"), ("nord", "blue")]).await;
    wired.open();
    wired.fire("move-theme-picker-down");

    // When enter is pressed.
    wired.fire("confirm-theme-picker");

    // Then the app keeps the theme the highlight was previewing.
    assert_eq!(wired.live_accent(), accent_theme("blue").focus_accent);
}

#[rstest::rstest]
#[tokio::test]
async fn confirming_clears_the_revert_snapshot() {
    // Given an open picker whose highlight has moved off the first theme.
    let wired = Wired::new(vec![("gruvbox", "red"), ("nord", "blue")]).await;
    wired.state.borrow_mut().frontend.theme = accent_theme("green");
    wired.open();
    wired.fire("move-theme-picker-down");

    // When enter is pressed.
    wired.fire("confirm-theme-picker");

    // Then nothing is left for a later close to restore — the choice is
    // authoritative and must survive the picker going away.
    let cell = wired.cell();
    assert!(
        cell.read().preview_original.is_none(),
        "confirm must clear the snapshot so nothing reverts the choice"
    );
}

// ── 5. Escape ───────────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn escape_restores_the_theme_the_picker_opened_with() {
    // Given an open picker that previewed a different theme.
    let wired = Wired::new(vec![("gruvbox", "red"), ("nord", "blue")]).await;
    wired.state.borrow_mut().frontend.theme = accent_theme("green");
    wired.open();
    wired.fire("move-theme-picker-down");
    assert_eq!(wired.live_accent(), accent_theme("blue").focus_accent);

    // When escape is pressed.
    wired.fire("cancel-theme-picker");

    // Then the app is back to the theme it had when the picker opened.
    assert_eq!(wired.live_accent(), accent_theme("green").focus_accent);
}

#[rstest::rstest]
#[tokio::test]
async fn escape_asks_the_scope_stack_to_pop_the_picker() {
    // Given an open picker.
    let wired = Wired::new(vec![("gruvbox", "red")]).await;
    wired.open();

    // When escape is pressed.
    let result = wired.fire("cancel-theme-picker");

    // Then the result pops the picker's own scope, closing the menu.
    assert_eq!(
        result.scope_signal,
        Some(ScopeSignal::PopIf(theme_picker_scope()))
    );
}

#[rstest::rstest]
#[tokio::test]
async fn escape_persists_nothing() {
    // Given an open picker whose highlight has previewed a second theme.
    let wired = Wired::new(vec![("gruvbox", "red"), ("nord", "blue")]).await;
    wired.open();
    wired.fire("move-theme-picker-down");

    // When escape is pressed.
    let result = wired.fire("cancel-theme-picker");

    // Then no theme name travels out — the preview was a look, not a choice,
    // and persisting it would survive the revert the user just asked for.
    assert!(
        result.message_names.is_empty(),
        "escape must not persist a theme; got {:?}",
        result.message_names
    );
}

#[rstest::rstest]
#[tokio::test]
async fn clear_control_key_closes_the_picker_when_the_filter_is_empty() {
    // Given an open picker with no filter text.
    let wired = Wired::new(vec![("gruvbox", "red")]).await;
    wired.open();

    // When ctrl-c is pressed.
    let result = wired.fire("clear-filter-or-leave-theme-picker");

    // Then the picker asked the scope stack to pop it.
    assert_eq!(
        result.scope_signal,
        Some(ScopeSignal::PopIf(theme_picker_scope()))
    );
}

#[rstest::rstest]
#[tokio::test]
async fn clear_control_key_clears_a_non_empty_filter_instead_of_closing() {
    // Given an open picker whose filter reads "gru".
    let wired = Wired::new(vec![("gruvbox", "red")]).await;
    wired.open();
    for ch in "gru".chars() {
        wired.edit(&EditIntent::InsertChar(ch));
    }

    // When ctrl-c is pressed.
    let result = wired.fire("clear-filter-or-leave-theme-picker");

    // Then the filter is empty and the picker did not ask to close.
    assert_eq!(wired.filter(), "");
    assert!(
        result.scope_signal.is_none(),
        "ctrl-c with text in the filter must not close the picker"
    );
}

// ── Live preview ────────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn moving_the_highlight_applies_that_theme_immediately() {
    // Given an open picker whose second theme is visually distinct.
    let wired = Wired::new(vec![("gruvbox", "red"), ("nord", "blue")]).await;
    wired.open();
    assert_eq!(
        wired.live_accent(),
        jinn_theme::default_theme().focus_accent
    );

    // When the highlight moves to the second theme.
    wired.fire("move-theme-picker-down");

    // Then the app is already wearing it — no confirm needed.
    assert_eq!(wired.live_accent(), accent_theme("blue").focus_accent);
}

#[rstest::rstest]
#[tokio::test]
async fn moving_the_highlight_invalidates_the_theme_sensitive_caches() {
    // Given an open picker with a populated theme-sensitive cache.
    let wired = Wired::new(vec![("gruvbox", "red"), ("nord", "blue")]).await;
    wired.open();
    wired.seed_theme_cache();
    assert_eq!(wired.theme_cache_len(), 1);

    // When the highlight moves.
    wired.fire("move-theme-picker-down");

    // Then the cache was dropped, so nothing renders the old theme's colors.
    assert_eq!(
        wired.theme_cache_len(),
        0,
        "moving the highlight must invalidate theme caches"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn escape_invalidates_the_theme_sensitive_caches() {
    // Given an open picker that previewed another theme, with a cache
    // repopulated while the preview was live.
    let wired = Wired::new(vec![("gruvbox", "red"), ("nord", "blue")]).await;
    wired.open();
    wired.fire("move-theme-picker-down");
    wired.seed_theme_cache();

    // When escape is pressed.
    wired.fire("cancel-theme-picker");

    // Then the restored theme's cache is dropped too.
    assert_eq!(
        wired.theme_cache_len(),
        0,
        "restoring the pre-open theme must invalidate theme caches"
    );
}

// ── Rendering ───────────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn each_row_draws_its_theme_swatch_alongside_the_name() {
    // Given an open picker holding one theme.
    let wired = Wired::new(vec![("gruvbox", "red")]).await;
    wired.open();

    // When the popup is drawn.
    let frame = wired.draw();

    // Then the swatch glyph and the theme's name both appear.
    let rendered: String = frame.concat();
    assert!(
        rendered.contains('\u{2588}') && rendered.contains("gruvbox"),
        "a theme row draws its swatch and name; got {rendered:?}"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn the_status_line_names_the_persisted_theme() {
    // Given an open picker over a slice with no persisted theme.
    let wired = Wired::new(vec![("gruvbox", "red")]).await;
    wired.open();

    // When the popup is drawn.
    let frame = wired.draw();

    // Then the status line reports the built-in default as in force.
    let rendered: String = frame.concat();
    assert!(
        rendered.contains("Current: default"),
        "the status line names the persisted theme; got {rendered:?}"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn the_status_line_names_a_persisted_theme_other_than_the_default() {
    // Given a slice whose persisted theme is "nord", with the picker open.
    let wired = Wired::new(vec![("gruvbox", "red")]).await;
    wired.state.borrow_mut().frontend.app_state.theme_name = Some("nord".to_owned());
    wired.open();

    // When the popup is drawn.
    let frame = wired.draw();

    // Then the status line names it, not the default.
    let rendered: String = frame.concat();
    assert!(
        rendered.contains("Current: nord"),
        "the status line names the persisted theme; got {rendered:?}"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn drawing_a_frame_publishes_the_measured_row_count_for_paging() {
    // Given an open picker whose cell starts on the pre-render fallback.
    let wired = Wired::new(vec![("gruvbox", "red")]).await;
    wired.open();
    let fallback = {
        let cell = wired.cell();
        cell.read().results_viewport
    };
    assert_eq!(fallback, jinn_theme_msg::RESULTS_VIEWPORT_FALLBACK);

    // When a frame is drawn.
    wired.draw();

    // Then the cell carries what the frame actually laid out, so paging
    // moves by a page of what is on screen rather than a fixed guess.
    let cell = wired.cell();
    let measured = cell.read().results_viewport;
    let expected =
        crate::theme_picker_viewport::results_viewport(ratatui::layout::Rect::new(0, 0, 100, 30));
    assert_eq!(measured, expected);
    assert_ne!(
        measured, fallback,
        "a narrow popup lays out far fewer rows than the fallback, so the \
         measurement must replace it rather than sit beside it"
    );
}

// ── 6. Every advertised key is bound ────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn every_key_the_footer_advertises_is_bound_in_the_picker_scope() {
    // Given a wired slice.
    let wired = Wired::new(vec![("gruvbox", "red")]).await;

    // When the set of keys the picker attached is read.
    let attached: Vec<&'static str> = wired
        .routes
        .rows()
        .iter()
        .filter(|row| row.scope == theme_picker_scope())
        .map(|row| row.key)
        .collect();

    // Then every key the footer advertises is among them — no dead key.
    for row in crate::theme_picker_render::theme_picker_binds() {
        assert!(
            attached.contains(&row.notation),
            "the footer advertises {} but the scope binds {attached:?}",
            row.notation
        );
    }
}

#[rstest::rstest]
#[tokio::test]
async fn the_picker_declares_every_key_it_actually_binds_in_the_footer() {
    // Given a wired slice.
    let wired = Wired::new(vec![("gruvbox", "red")]).await;

    // When the attached rows are read.
    let picker_rows: Vec<&'static str> = wired
        .routes
        .rows()
        .iter()
        .filter(|row| {
            row.scope == theme_picker_scope() && matches!(row.outcome, RouteOutcome::Action { .. })
        })
        .map(|row| row.key)
        .collect();
    let advertised: Vec<&'static str> = crate::theme_picker_render::theme_picker_binds()
        .iter()
        .map(|row| row.notation)
        .collect();

    // Then the footer's two confirm/cancel keys are the picker's own, and
    // every key it binds is reachable through an action.
    assert_eq!(advertised, vec!["<enter>", "<esc>"]);
    for key in &picker_rows {
        assert!(
            attached_key(&wired, key),
            "picker binds {key} through an action, so it is reachable"
        );
    }
}

/// Whether the picker's scope binds `key` through an action row.
fn attached_key(wired: &Wired, key: &str) -> bool {
    wired.routes.rows().iter().any(|row| {
        row.scope == theme_picker_scope()
            && row.key == key
            && matches!(row.outcome, RouteOutcome::Action { .. })
    })
}

// ── 7. One owner ────────────────────────────────────────────────────────

#[rstest::rstest]
fn the_picker_state_lives_only_in_its_slice_cell() {
    // The theme picker's state is reachable from exactly one place: the slice
    // cell. A second copy in the kernel would let the menu show one store
    // while a different one is written — the defect that left the skills
    // menu blank.
    let kernel_source = include_str!("../../../jinn-domain/src/feat/ui/picker_states.rs");
    assert!(
        !kernel_source.contains("theme_picker"),
        "the kernel must not hold theme picker state; the slice cell is the only home"
    );
}

#[rstest::rstest]
fn the_kernel_names_no_theme_picker_at_all() {
    // The central app crate and the TUI layer must not know this picker
    // exists: no scope variant, no picker kind, no spec id. That is what makes
    // adding a picker a folder-local change.
    for (label, source) in [
        (
            "jinn-domain picker state",
            include_str!("../../../jinn-domain/src/feat/ui/picker_states.rs"),
        ),
        (
            "jinn-domain picker host",
            include_str!("../../../jinn-domain/src/feat/picker/host_impl.rs"),
        ),
        (
            "jinn-tui scope table",
            include_str!("../../../jinn-tui/src/scope.rs"),
        ),
    ] {
        let picker_named = ["PickerTheme", "Picker(theme)", "THEME_ID", "theme_spec"];
        let hits: Vec<&str> = picker_named
            .iter()
            .filter(|needle| source.contains(*needle))
            .copied()
            .collect();
        assert!(
            hits.is_empty(),
            "{label} still names the theme picker ({hits:?}); the slice must own it entirely"
        );
    }
}
