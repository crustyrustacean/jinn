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

//! The session-lifecycle picker's observable behavior.
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

use jinn_preferences_config::schemas::{LifecycleCommand, SessionLifecycle};
use jinn_session_lifecycle_msg::picker_state::SessionLifecyclePickerState;
use jinn_session_lifecycle_msg::session_lifecycle_picker_scope;
use jinn_slices::cell::TypedCell;
use jinn_slices::route::{EditIntent, ScopeSignal};
use jinn_slices::{KeyRoutes, SliceHost, Slices};

/// The route action name that opens the picker.
const OPEN_ACTION: &str = "open-session-lifecycle-picker";

/// Renders `lifecycles` as the `[[session_lifecycle.script]]` document the
/// configuration layer parses. The picker reads its rows through the layer
/// rather than through kernel frontend state, so the test seeds the layer.
fn lifecycle_document(lifecycles: &[SessionLifecycle]) -> String {
    use std::fmt::Write as _;

    let mut document = String::new();
    for lifecycle in lifecycles {
        document.push_str("[[session_lifecycle.script]]\n");
        let _ = writeln!(document, "name = \"{}\"", lifecycle.name);
        if let Some(LifecycleCommand::Shell(shell)) = &lifecycle.setup {
            let _ = writeln!(document, "setup_command = \"{shell}\"");
        }
    }
    document
}

/// The slice wired exactly as composition wires it.
struct Wired {
    slices: Slices,
    routes: KeyRoutes,
    state: std::cell::RefCell<jinn_domain::AppState>,
    /// Kept alive for the test's lifetime: the route actions read lifecycles
    /// through the configuration layer, which borrows the document.
    config: jinn_config::ConfigLayer,
}

impl Wired {
    /// Builds the picker over `lifecycles`, with the argument popup's cell
    /// registered as `activate` does.
    async fn new(lifecycles: Vec<SessionLifecycle>) -> Self {
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
            slices
                .register(
                    jinn_session_lifecycle_msg::arg_input_slot(),
                    jinn_session_lifecycle_msg::ArgInputState::empty(),
                )
                .expect("fresh registry has the argument slot free");
            crate::activate_picker(&mut host);
        }
        // `default_with_scope_focus`, not `default`: a scope push is a no-op
        // without the shared scope cell, and this test asserts on the stack.
        let state = jinn_domain::AppState::default_with_scope_focus();
        state.frontend.attach_slices(slices.clone());
        let config = jinn_config::testutil::config_layer(&lifecycle_document(&lifecycles));
        Self {
            slices,
            routes,
            state: std::cell::RefCell::new(state),
            config,
        }
    }

    /// The picker cell.
    fn cell(&self) -> TypedCell<SessionLifecyclePickerState> {
        self.slices
            .reader(&jinn_session_lifecycle_msg::session_lifecycle_picker_slot())
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

    /// The result-row count the last frame measured.
    fn measured_viewport(&self) -> usize {
        self.cell().read().results_viewport
    }

    /// Opens the picker the way the kernel does: dispatch the row's action,
    /// then apply the scope signal it requested, firing the picker's
    /// scope-enter hook.
    ///
    /// The hook is what seeds the rows and clears the filter, so a test that
    /// only fired the action would exercise a picker that never opened.
    fn open(&self) {
        let result = self.fire(OPEN_ACTION);
        self.apply_signal(
            result
                .scope_signal
                .expect("the open action requests a push"),
        );
    }

    /// Applies a scope signal the way the kernel does, so a slice's
    /// scope-enter hook fires on a push.
    fn apply_signal(&self, signal: ScopeSignal) {
        let mut state = self.state.borrow_mut();
        match signal {
            ScopeSignal::Push(id) => {
                state
                    .frontend
                    .scope_push(jinn_slices::FocusScope::Dynamic(id.clone()));
                if let Some(hook) = self.routes.scope_enter_hook(&id) {
                    hook(jinn_slices::ActionCtx {
                        state: &mut *state,
                        slices: &self.slices,
                        config: &self.config,
                        key_bytes: Vec::new(),
                    });
                }
            }
            ScopeSignal::PopIf(id) => {
                if matches!(&state.frontend.scope(), jinn_slices::FocusScope::Dynamic(cur) if *cur == id)
                {
                    state.frontend.scope_pop();
                }
            }
        }
    }

    /// Dispatches one of the picker's actions by its route action name.
    fn fire(&self, action: &str) -> jinn_slices::RouteResult {
        let mut state = self.state.borrow_mut();
        self.routes
            .action_for(
                &jinn_slices::DynamicIntent::new(session_lifecycle_picker_scope(), action, action),
                jinn_slices::ActionCtx {
                    state: &mut *state,
                    slices: &self.slices,
                    config: &self.config,
                    key_bytes: Vec::new(),
                },
            )
            .unwrap_or_else(|| {
                panic!("the session-lifecycle picker attaches an action named {action}")
            })
    }

    /// Feeds an edit intent to the picker's registered input hook.
    fn edit(&self, intent: &EditIntent) {
        let hook = self
            .routes
            .input_hook(&session_lifecycle_picker_scope())
            .expect("the picker registers an input hook for its filter");
        hook(intent).expect("the filter hook always consumes the edit");
    }

    /// Draws one frame of the picker and returns the buffer's symbols.
    fn draw(&self, width: u16, height: u16) -> Vec<String> {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let area = ratatui::layout::Rect::new(0, 0, width, height);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        let facts =
            jinn_slices::RenderFacts::new(self.state.borrow().frontend.theme.clone(), &self.slices);
        terminal
            .draw(|frame| {
                let popup =
                    crate::session_lifecycle_picker_render::session_lifecycle_picker_overlay_rect(
                        &area,
                    )
                    .expect("geometry fn yields a popup rect");
                crate::session_lifecycle_picker_render::render_session_lifecycle_picker(
                    frame, popup, &facts,
                );
            })
            .expect("draw");
        terminal
            .backend()
            .buffer()
            .content
            .chunks(width as usize)
            .map(|row| row.iter().map(ratatui::buffer::Cell::symbol).collect())
            .collect()
    }
}

/// A lifecycle with no setup command — confirming it starts the session
/// immediately.
fn plain(name: &str) -> SessionLifecycle {
    SessionLifecycle {
        name: name.to_owned(),
        description: Some(format!("the {name} lifecycle")),
        setup: None,
        teardown: None,
    }
}

/// A lifecycle whose setup shell command takes a `$`-parameter, so confirming
/// it must hand off to the argument popup.
fn parametrized(name: &str, command: &str) -> SessionLifecycle {
    SessionLifecycle {
        name: name.to_owned(),
        description: None,
        setup: Some(LifecycleCommand::Shell(command.to_owned())),
        teardown: None,
    }
}

// ── 1. Opens with entries ───────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn opening_the_picker_lists_the_blank_lifecycle_first() {
    // Given a picker with no configured lifecycles.
    let wired = Wired::new(vec![]).await;

    // When the picker opens.
    wired.open();

    // Then the implicit blank lifecycle is the only row.
    assert_eq!(wired.visible_names(), vec!["blank".to_owned()]);
}

#[rstest::rstest]
#[tokio::test]
async fn opening_the_picker_lists_every_configured_lifecycle_after_blank() {
    // Given two configured lifecycles.
    let wired = Wired::new(vec![plain("dev"), plain("review")]).await;

    // When the picker opens.
    wired.open();

    // Then every lifecycle is a row, with blank leading.
    assert_eq!(
        wired.visible_names(),
        vec!["blank".to_owned(), "dev".to_owned(), "review".to_owned()]
    );
}

#[rstest::rstest]
#[tokio::test]
async fn opening_the_picker_pushes_its_own_scope() {
    // Given a wired picker.
    let wired = Wired::new(vec![]).await;

    // When the open action is dispatched.
    let result = wired.fire(OPEN_ACTION);

    // Then the picker's scope lands on the stack.
    assert_eq!(
        result.scope_signal,
        Some(ScopeSignal::Push(session_lifecycle_picker_scope())),
        "opening must push the picker's own scope"
    );
}

// ── 2. Filter narrows ───────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn reopening_the_picker_clears_the_filter() {
    // Given a picker that was opened and then filtered.
    let wired = Wired::new(vec![plain("dev"), plain("review")]).await;
    wired.open();
    wired.edit(&EditIntent::InsertChar('r'));
    assert_eq!(wired.filter(), "r", "the filter is set before reopening");

    // When the picker is opened again.
    wired.open();

    // Then the filter is empty — every open starts fresh.
    assert_eq!(wired.filter(), "");
}

#[rstest::rstest]
#[tokio::test]
async fn reopening_the_picker_moves_the_highlight_back_to_the_first_row() {
    // Given a picker that was opened and then navigated.
    let wired = Wired::new(vec![plain("dev"), plain("review")]).await;
    wired.open();
    wired.fire("move-lifecycle-picker-down");
    assert_eq!(
        wired.highlighted(),
        1,
        "the highlight moved before reopening"
    );

    // When the picker is opened again.
    wired.open();

    // Then the highlight is back on the first row.
    assert_eq!(wired.highlighted(), 0);
}

#[rstest::rstest]
#[tokio::test]
async fn typing_a_character_filters_the_list() {
    // Given a picker over three lifecycles.
    let wired = Wired::new(vec![plain("dev"), plain("review"), plain("release")]).await;
    wired.open();

    // When a character is typed into the filter.
    wired.edit(&EditIntent::InsertChar('r'));

    // Then only the matching lifecycles remain.
    assert_eq!(
        wired.visible_names(),
        vec!["review".to_owned(), "release".to_owned()]
    );
}

#[rstest::rstest]
#[tokio::test]
async fn the_filter_also_matches_a_lifecycles_description() {
    // Given a picker whose second lifecycle is described by a word absent
    // from its name.
    let wired = Wired::new(vec![plain("alpha"), plain("beta")]).await;
    wired.open();

    // When the filter matches on the description word "beta".
    for ch in "beta".chars() {
        wired.edit(&EditIntent::InsertChar(ch));
    }

    // Then the description-only match is still shown, and the other is not.
    assert_eq!(
        wired.visible_names(),
        vec!["beta".to_owned()],
        "a lifecycle must be findable by its description text"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn backspace_shortens_the_filter() {
    // Given a picker with two characters of filter.
    let wired = Wired::new(vec![plain("dev"), plain("review")]).await;
    wired.open();
    wired.edit(&EditIntent::InsertChar('r'));
    wired.edit(&EditIntent::InsertChar('e'));

    // When backspace removes one.
    wired.edit(&EditIntent::DeleteBackward);

    // Then only the first character remains.
    assert_eq!(wired.filter(), "r");
}

// ── 3. Navigation ───────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn down_moves_the_highlight_to_the_next_row() {
    // Given a picker over three lifecycles.
    let wired = Wired::new(vec![plain("dev"), plain("review")]).await;
    wired.open();

    // When `<down>` is pressed.
    wired.fire("move-lifecycle-picker-down");

    // Then the highlight advanced one row.
    assert_eq!(wired.highlighted(), 1);
}

#[rstest::rstest]
#[tokio::test]
async fn up_from_the_first_row_stays_on_the_first_row() {
    // Given a picker with the highlight on the first row.
    let wired = Wired::new(vec![plain("dev"), plain("review")]).await;
    wired.open();

    // When `<up>` is pressed.
    wired.fire("move-lifecycle-picker-up");

    // Then the highlight does not wrap backwards.
    assert_eq!(wired.highlighted(), 0);
}

#[rstest::rstest]
#[tokio::test]
async fn drawing_a_frame_publishes_the_measured_row_count_for_paging() {
    // Given a picker over many lifecycles.
    let wired = Wired::new((0..40).map(|i| plain(&format!("life{i:02}"))).collect()).await;
    wired.open();

    // When a frame is drawn.
    let _ = wired.draw(100, 16);

    // Then the cell holds the measurement, not the fallback.
    assert_ne!(
        wired.measured_viewport(),
        jinn_session_lifecycle_msg::RESULTS_VIEWPORT_FALLBACK,
        "the render pass must publish its own measurement for paging"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn page_down_steps_by_the_rows_the_last_frame_actually_laid_out() {
    // Given a picker over many lifecycles, with a frame drawn so the
    // measurement is known.
    let wired = Wired::new((0..40).map(|i| plain(&format!("life{i:02}"))).collect()).await;
    wired.open();
    let _ = wired.draw(100, 16);
    let viewport = wired.measured_viewport();
    let expected = (viewport / 2).max(1);

    // When `<pgdn>` is pressed.
    wired.fire("page-lifecycle-picker-down");

    // Then the highlight advanced by the measured window, not the fallback.
    assert_eq!(wired.highlighted(), expected);
    assert_ne!(
        viewport,
        jinn_session_lifecycle_msg::RESULTS_VIEWPORT_FALLBACK
    );
}

// ── 4. Confirm ──────────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn confirming_a_plain_lifecycle_starts_the_session() {
    // Given a picker over a setup-less lifecycle.
    let wired = Wired::new(vec![plain("dev")]).await;
    wired.open();
    wired.fire("move-lifecycle-picker-down");

    // When `<enter>` is pressed.
    let result = wired.fire("confirm-session-lifecycle-picker");

    // Then the confirm ran without asking for arguments.
    assert_ne!(
        result.scope_signal,
        Some(ScopeSignal::Push(
            jinn_session_lifecycle_msg::arg_input_scope()
        )),
        "a setup-less lifecycle must not hand off to the argument popup"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn confirming_a_parametrized_lifecycle_hands_off_to_the_argument_popup() {
    // Given a picker over a lifecycle whose setup takes a parameter.
    let wired = Wired::new(vec![parametrized("deploy", "echo deploy $1")]).await;
    wired.open();
    wired.fire("move-lifecycle-picker-down");

    // When `<enter>` is pressed.
    let result = wired.fire("confirm-session-lifecycle-picker");

    // Then the argument popup is pushed instead of starting anything.
    assert_eq!(
        result.scope_signal, None,
        "the handoff is applied to the scope stack, not signalled"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn confirming_a_parametrized_lifecycle_lands_on_the_argument_popup() {
    // Given an open picker over a parameterized lifecycle.
    let wired = Wired::new(vec![parametrized("deploy", "echo deploy $1")]).await;
    wired.open();
    wired
        .state
        .borrow()
        .frontend
        .scope_push(jinn_slices::FocusScope::Dynamic(
            session_lifecycle_picker_scope(),
        ));
    wired.fire("move-lifecycle-picker-down");

    // When `<enter>` is pressed.
    let _ = wired.fire("confirm-session-lifecycle-picker");

    // Then the top of the scope stack is the argument popup, not the picker —
    // the picker was popped, so the popup sits directly beneath nothing.
    let top = wired.state.borrow().frontend.scope().clone();
    assert!(
        matches!(
            &top,
            jinn_slices::FocusScope::Dynamic(scope)
                if *scope == jinn_session_lifecycle_msg::arg_input_scope()
        ),
        "the argument popup must end up on top, got {top:?}"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn confirming_a_parametrized_lifecycle_seeds_the_argument_cell() {
    // Given a picker over a parameterized lifecycle.
    let wired = Wired::new(vec![parametrized("deploy", "echo deploy $1")]).await;
    wired.open();
    wired.fire("move-lifecycle-picker-down");

    // When `<enter>` is pressed.
    let _ = wired.fire("confirm-session-lifecycle-picker");

    // Then the argument cell holds the lifecycle's template.
    let cell = wired
        .slices
        .reader::<jinn_session_lifecycle_msg::ArgInputState>(
            &jinn_session_lifecycle_msg::arg_input_slot(),
        )
        .expect("the argument slot is registered at activation");
    let guard = cell.read();
    assert_eq!(
        guard.lifecycle_name, "deploy",
        "the popup must be seeded with the chosen lifecycle"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn confirming_with_nothing_highlighted_does_nothing() {
    // Given a picker over no lifecycles at all, opened and left unpopulated.
    let wired = Wired::new(vec![]).await;
    wired.open();
    wired
        .cell()
        .update(|state| state.selection.set_items(vec![]));

    // When `<enter>` is pressed.
    let result = wired.fire("confirm-session-lifecycle-picker");

    // Then nothing is pushed and nothing is popped.
    assert_eq!(
        result.scope_signal, None,
        "an empty picker must be inert, got {:?}",
        result.scope_signal
    );
}

// ── 5. Escape ───────────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn escape_asks_the_scope_stack_to_pop_the_picker() {
    // Given an open picker.
    let wired = Wired::new(vec![]).await;
    wired.open();

    // When `<esc>` is pressed.
    let result = wired.fire("cancel-session-lifecycle-picker");

    // Then the picker scope is popped.
    assert_eq!(
        result.scope_signal,
        Some(ScopeSignal::PopIf(session_lifecycle_picker_scope())),
        "escape must pop the picker"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn escape_starts_no_lifecycle() {
    // Given an open picker with a lifecycle highlighted.
    let wired = Wired::new(vec![parametrized("deploy", "echo $1")]).await;
    wired.open();
    wired.fire("move-lifecycle-picker-down");

    // When `<esc>` is pressed.
    let result = wired.fire("cancel-session-lifecycle-picker");

    // Then the argument popup is never pushed — escape changes nothing.
    assert_ne!(
        result.scope_signal,
        Some(ScopeSignal::Push(
            jinn_session_lifecycle_msg::arg_input_scope()
        )),
        "escape must not start or hand off to anything"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn clear_control_key_clears_a_non_empty_filter_instead_of_closing() {
    // Given an open picker with filter text.
    let wired = Wired::new(vec![plain("dev"), plain("review")]).await;
    wired.open();
    wired.edit(&EditIntent::InsertChar('r'));

    // When `<c-c>` is pressed.
    let result = wired.fire("clear-filter-or-leave-lifecycle-picker");

    // Then the filter is emptied and the picker stays open.
    assert_eq!(wired.filter(), "", "ctrl-c must clear the filter first");
    assert_eq!(
        result.scope_signal, None,
        "ctrl-c with filter text must not close the picker, got {:?}",
        result.scope_signal
    );
}

#[rstest::rstest]
#[tokio::test]
async fn clear_control_key_closes_the_picker_when_the_filter_is_already_empty() {
    // Given an open picker with no filter text.
    let wired = Wired::new(vec![]).await;
    wired.open();

    // When `<c-c>` is pressed.
    let result = wired.fire("clear-filter-or-leave-lifecycle-picker");

    // Then the picker closes.
    assert_eq!(
        result.scope_signal,
        Some(ScopeSignal::PopIf(session_lifecycle_picker_scope())),
        "ctrl-c on an empty filter must close the picker"
    );
}

// ── 6. No dead keys advertised ──────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn every_key_the_footer_advertises_is_bound_in_the_picker_scope() {
    // Given a wired picker.
    let wired = Wired::new(vec![]).await;

    // When the set of keys the picker attached is read.
    let attached: Vec<&'static str> = wired
        .routes
        .rows()
        .iter()
        .filter(|row| row.scope == session_lifecycle_picker_scope())
        .map(|row| row.key)
        .collect();

    // Then every key the footer advertises is among them — no dead key.
    for row in crate::session_lifecycle_picker_render::session_lifecycle_picker_binds() {
        assert!(
            attached.contains(&row.notation),
            "the footer advertises {} but the scope binds {attached:?}",
            row.notation
        );
    }
}

#[rstest::rstest]
fn the_footer_advertises_the_confirm_and_cancel_keys() {
    // Given the picker's declared footer bindings.
    let bindings = crate::session_lifecycle_picker_routes::SESSION_LIFECYCLE_PICKER_BINDINGS;

    // When the notations are collected.
    let notations: Vec<&str> = bindings.iter().map(|(notation, _)| *notation).collect();

    // Then confirm and cancel are both present, and nothing is padding.
    assert_eq!(notations, vec!["<enter>", "<esc>"]);
}

#[rstest::rstest]
#[tokio::test]
async fn every_base_key_resolves_through_a_slice_action() {
    // Given the picker's wired routes.
    let wired = Wired::new(vec![]).await;

    // When each base key is fired by its action name.
    for (action, key) in [
        ("confirm-session-lifecycle-picker", "<enter>"),
        ("cancel-session-lifecycle-picker", "<esc>"),
        ("move-lifecycle-picker-up", "<up>"),
        ("move-lifecycle-picker-down", "<down>"),
        ("page-lifecycle-picker-up", "<pgup>"),
        ("page-lifecycle-picker-down", "<pgdn>"),
        ("clear-filter-or-leave-lifecycle-picker", "<c-c>"),
    ] {
        // Then the action exists, so the key is not a silently-dropped row.
        assert!(
            wired
                .routes
                .action_for(
                    &jinn_slices::DynamicIntent::new(
                        session_lifecycle_picker_scope(),
                        action,
                        action,
                    ),
                    jinn_slices::ActionCtx {
                        state: &mut *wired.state.borrow_mut(),
                        slices: &wired.slices,
                        config: jinn_slices::empty_config_layer(),
                        key_bytes: Vec::new(),
                    },
                )
                .is_some(),
            "{key} must resolve to the slice action {action}"
        );
    }
}

#[rstest::rstest]
#[tokio::test]
async fn the_filter_hook_is_registered_for_the_picker_scope() {
    // Given the wired picker.
    let wired = Wired::new(vec![]).await;

    // When the input hook is looked up.
    let hook = wired.routes.input_hook(&session_lifecycle_picker_scope());

    // Then it is present, so typing in the filter works.
    assert!(
        hook.is_some(),
        "without an input hook the filter would silently stop accepting text"
    );
}

// ── 7. One owner ────────────────────────────────────────────────────────

#[rstest::rstest]
fn the_picker_state_lives_only_in_its_slice_cell() {
    // Given the kernel's picker state block.
    let kernel = include_str!("../../../jinn-domain/src/feat/ui/frontend_state.rs");

    // When it is searched for this picker's state.
    let found = kernel.contains("session_lifecycle_picker");

    // Then the kernel holds no copy: one home, or the menu opens empty.
    assert!(
        !found,
        "the kernel must not hold session-lifecycle picker state; the slice \
         cell is the single home"
    );
}

#[rstest::rstest]
fn the_kernel_names_no_session_lifecycle_picker() {
    // Given the central crates' sources.
    let sources = [
        include_str!("../../../jinn-domain/src/feat/intent/handler.rs"),
        include_str!("../../../jinn-domain/src/protocol/intent.rs"),
    ];

    // When each is searched for a picker identity.
    for (i, source) in sources.iter().enumerate() {
        // Then the kernel names no scope variant, kind, or id for it.
        for needle in [
            "PickerKind::SessionLifecycle",
            "Scope::PickerLifecycle",
            "SESSION_LIFECYCLE_ID",
        ] {
            assert!(
                !source.contains(needle),
                "central source {i} still names the slice-owned picker: {needle}"
            );
        }
    }
}
