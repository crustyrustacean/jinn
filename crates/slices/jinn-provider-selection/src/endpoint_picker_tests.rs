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

//! The OpenRouter endpoint picker's observable behavior.
//!
//! Wiring assertions run the slice's real activation, so a picker that
//! registered its cell but forgot its rows, its overlay, or its filter hook
//! fails loudly instead of passing against a hand-built stand-in. The
//! provider actors are *not* spawned: these tests describe the picker, not the
//! fetch machinery.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test module, panics are acceptable"
)]

use jinn_core_types::model_selection::{AlloyStrategy, ModelSelection};
use jinn_provider_selection_msg::SetEndpointDefault;
use jinn_provider_selection_msg::endpoint::EndpointEntry;
use jinn_provider_selection_msg::endpoint::EndpointPickerState;
use jinn_provider_selection_msg::endpoint::endpoint_picker_scope;
use jinn_provider_selection_msg::endpoint::endpoint_picker_slot;
use jinn_slices::cell::TypedCell;
use jinn_slices::route::{EditIntent, ScopeSignal};
use jinn_slices::{KeyRoutes, SliceHost, Slices};

/// The slice wired exactly as composition wires it.
struct Wired {
    slices: Slices,
    routes: KeyRoutes,
    state: std::cell::RefCell<jinn_kernel::AppState>,
}

impl Wired {
    /// Builds the slice with the endpoint picker registered.
    async fn new() -> Self {
        let slices = Slices::new();
        let mut viewport = jinn_slices::view::Viewport::new();
        let overlay_views = jinn_slices::OverlayViews::new();
        let routes = KeyRoutes::new();
        let services = jinn_kernel::Services::new_fake().await;
        {
            let mut host = SliceHost::new(
                &slices,
                &mut viewport,
                &overlay_views,
                &routes,
                &services.trouper_system,
            );
            // The provider cell is part of this slice's real activation: the
            // picker reads it for the fetch flags its status line shows.
            host.register_cell(
                jinn_provider_selection_msg::provider_state_slot(),
                jinn_provider_selection_msg::ProviderCell::default(),
            )
            .expect("the provider cell is registered once at wiring");
            let cell = host
                .register_cell(endpoint_picker_slot(), EndpointPickerState::default())
                .expect("the endpoint picker cell is registered once at wiring");
            crate::activate_endpoint_picker(&mut host, &cell);
        }
        // Mirror `jinn_tui::app::builder`: seed the scope-focus slot on *this*
        // registry, then attach that same registry. `attach_slices` writes a
        // `OnceLock`, so a state built by `default_with_scope_focus` (which
        // mints its own registry) would keep the other one and the picker
        // could never reach the provider cell.
        let _ = slices.register(
            jinn_slices::scope_focus_slot(),
            jinn_slices::ScopeFocusState::default(),
        );
        let state = jinn_kernel::AppState::default();
        state.frontend.attach_slices(slices.clone());
        Self {
            slices,
            routes,
            state: std::cell::RefCell::new(state),
        }
    }

    /// The picker cell.
    fn cell(&self) -> TypedCell<EndpointPickerState> {
        self.slices
            .reader(&endpoint_picker_slot())
            .expect("the picker registers its cell at activation")
    }

    /// The provider names the filter currently shows, in display order.
    ///
    /// Reads the *filtered* view, not the underlying item list: a picker whose
    /// filter text is written but never applied still has every item present.
    fn visible(&self) -> Vec<String> {
        let cell = self.cell();
        let guard = cell.read();
        (0..guard.selection.filtered_count())
            .filter_map(|i| guard.selection.filtered_item(i))
            .map(|item| item.entry().provider_name.clone())
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

    /// Puts the active session on a named provider's model, the shape the
    /// picker is reachable for.
    fn set_single_model(&self) {
        self.state
            .borrow_mut()
            .active_session_mut()
            .profile_mut()
            .model = ModelSelection::Single("openrouter/anthropic/claude-sonnet-4.5".to_owned());
    }

    /// Puts the active session on an alloy model, the shape the picker must
    /// refuse.
    fn set_alloy_model(&self) {
        self.state
            .borrow_mut()
            .active_session_mut()
            .profile_mut()
            .model = ModelSelection::Alloy {
            models: vec!["openrouter/a".to_owned(), "openrouter/b".to_owned()],
            strategy: AlloyStrategy::RoundRobin { index: 0 },
        };
    }

    /// The `SetEndpointDefault` a result publishes, if it publishes one.
    ///
    /// Confirming no longer writes anything itself — an `ActionCtx` cannot
    /// reach `ConfigStorage` — so the observable effect of the key is the
    /// command it hands the provider actor.
    fn published_pin(result: jinn_slices::RouteResult) -> Option<SetEndpointDefault> {
        #[derive(Default)]
        struct RecordingSink {
            published: std::sync::Mutex<Vec<(String, serde_json::Value)>>,
        }

        impl jinn_slices::PublishSink for RecordingSink {
            fn publish_schema(
                &self,
                schema_id: trouper::schema::SchemaId,
                payload: serde_json::Value,
                _name: &'static str,
            ) {
                self.published
                    .lock()
                    .expect("sink lock")
                    .push((format!("{schema_id}"), payload));
            }
        }

        let sink = RecordingSink::default();
        for closure in result.messages {
            closure(&sink);
        }
        let published = sink.published.lock().expect("sink lock");
        let (_, payload) = published
            .iter()
            .find(|(id, _)| id.ends_with("SetEndpointDefault"))?;
        Some(
            serde_json::from_value(payload.clone())
                .expect("the published payload is a SetEndpointDefault"),
        )
    }

    /// Fills the picker with `entries` through its own actions.
    fn with_entries(&self, entries: Vec<EndpointEntry>) {
        let theme = self.state.borrow().frontend.theme.clone();
        self.cell()
            .update(|picker| crate::endpoint_picker_actions::open(picker, entries, &theme));
    }

    /// Opens the picker through its real `open` route action.
    fn open(&self) -> jinn_slices::RouteResult {
        self.fire("open-endpoint-picker")
    }

    /// Dispatches one of the picker's actions by its route action name.
    fn fire(&self, action: &str) -> jinn_slices::RouteResult {
        let mut state = self.state.borrow_mut();
        self.routes
            .action_for(
                &jinn_slices::DynamicIntent::new(endpoint_picker_scope(), action, action),
                jinn_slices::ActionCtx {
                    state: &mut *state,
                    slices: &self.slices,
                    config: jinn_slices::empty_config_layer(),
                    key_bytes: Vec::new(),
                },
            )
            .unwrap_or_else(|| panic!("the endpoint picker attaches an action named {action}"))
    }

    /// Feeds an edit intent to the picker's registered input hook.
    fn edit(&self, intent: &EditIntent) {
        let hook = self
            .routes
            .input_hook(&endpoint_picker_scope())
            .expect("the picker registers an input hook for its filter");
        hook(intent).expect("the filter hook always consumes the edit");
    }

    /// Types `text` into the filter, one character at a time.
    fn type_text(&self, text: &str) {
        for ch in text.chars() {
            self.edit(&EditIntent::InsertChar(ch));
        }
    }

    /// Draws one frame of the picker at `120x34` and returns the buffer's
    /// symbols, row by row.
    fn draw(&self) -> Vec<String> {
        draw_frame(self, ratatui::layout::Rect::new(0, 0, 120, 34))
    }
}

/// Draws one frame of the picker's popup into a fresh test terminal and
/// returns the buffer's symbols, row by row.
fn draw_frame(wired: &Wired, area: ratatui::layout::Rect) -> Vec<String> {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))
        .expect("a test terminal allocates");
    let facts =
        jinn_slices::RenderFacts::new(wired.state.borrow().frontend.theme.clone(), &wired.slices);
    terminal
        .draw(|frame| {
            let popup = crate::endpoint_picker_render::endpoint_picker_overlay_rect(&area)
                .expect("geometry fn yields a popup rect");
            crate::endpoint_picker_render::render_endpoint_picker(frame, popup, &facts);
        })
        .expect("the draw closure must not panic");
    terminal
        .backend()
        .buffer()
        .content
        .chunks(area.width as usize)
        .map(|row| row.iter().map(ratatui::buffer::Cell::symbol).collect())
        .collect()
}

/// One endpoint row.
fn entry(tag: &str, provider: &str, active: bool) -> EndpointEntry {
    let mut entry = EndpointEntry::auto_route(false, jinn_theme::default_theme());
    entry.tag = tag.to_owned();
    entry.provider_name = provider.to_owned();
    entry.is_active = active;
    entry
}

/// A representative three-row list: the auto-route sentinel, then two upstreams.
fn sample_entries() -> Vec<EndpointEntry> {
    vec![
        entry("", "auto-route", true),
        entry("us-east", "Acme", false),
        entry("eu-west", "Borealis", false),
    ]
}

// ── Wiring ──────────────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn open_pushes_the_pickers_own_scope() {
    // Given the slice wired with a single (non-alloy) model.
    let wired = Wired::new().await;
    wired.set_single_model();

    // When opening the picker through its route action.
    let result = wired.open();

    // Then the result pushes the picker's own dynamic scope.
    assert_eq!(
        result.scope_signal,
        Some(ScopeSignal::Push(endpoint_picker_scope())),
        "opening must push the scope this slice registered, not a kernel scope"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn open_requests_the_endpoint_fetch() {
    // Given the slice wired with a single model.
    let wired = Wired::new().await;
    wired.set_single_model();

    // When opening the picker.
    let result = wired.open();

    // Then a load-entries message is published for the provider actor.
    assert!(
        result
            .message_names
            .iter()
            .any(|name| name.ends_with("LoadEndpointPickerEntries")),
        "opening must ask the provider actor to load the upstream list, got {:?}",
        result.message_names
    );
}

#[rstest::rstest]
#[tokio::test]
async fn open_flags_the_fetch_in_flight() {
    // Given the slice wired with a single model.
    let wired = Wired::new().await;
    wired.set_single_model();

    // When opening the picker.
    wired.open();

    // Then the provider cell reports a fetch in progress, so the status line
    // shows activity rather than the previous fetch's age.
    let provider: TypedCell<jinn_provider_selection_msg::ProviderCell> = wired
        .slices
        .reader(&jinn_provider_selection_msg::provider_state_slot())
        .expect("the provider cell is registered at activation");
    assert!(
        provider.read().endpoint_loading,
        "the picker must show the fetch is in flight the moment it opens"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn the_filter_is_bound_in_the_pickers_own_scope() {
    // Given the slice wired as composition wires it.
    let wired = Wired::new().await;

    // When asking whether a character reaches the picker.
    let routes = &wired.routes;

    // Then the scope has an input hook, which the composition keymap
    // materializes into the printable-character catch-all.
    assert!(
        routes.input_hook(&endpoint_picker_scope()).is_some(),
        "without an input hook the filter silently stops accepting typed text"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn the_footer_advertises_only_keys_the_picker_binds() {
    // Given the picker's declared bindings.
    let binds = crate::endpoint_picker_render::endpoint_picker_binds();

    // When checking each one against the rows the slice actually attached.
    let wired = Wired::new().await;
    let attached: Vec<String> = wired
        .routes
        .rows()
        .into_iter()
        .filter(|row| row.scope == endpoint_picker_scope())
        .map(|row| row.key.to_owned())
        .collect();

    // Then every advertised key is a real binding: a footer must not promise
    // a key the picker does not answer to.
    for bind in &binds {
        assert!(
            attached.iter().any(|key| key == bind.notation),
            "footer advertises {} but no row binds it; attached: {attached:?}",
            bind.notation
        );
    }
}

// ── The model-shape gate ────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn open_declines_for_an_alloy_model() {
    // Given the slice wired with an alloy model, whose upstream routing is
    // chosen provider-side.
    let wired = Wired::new().await;
    wired.set_alloy_model();

    // When opening the picker.
    let result = wired.open();

    // Then nothing is pushed: the gate lives in the slice's own open action,
    // so it runs before the push rather than beside it.
    assert_eq!(
        result.scope_signal, None,
        "an alloy cannot be routed through a pinned endpoint, so the picker must not open"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn refresh_declines_for_an_alloy_model() {
    // Given the slice wired with an alloy model.
    let wired = Wired::new().await;
    wired.set_alloy_model();

    // When forcing a refresh.
    let result = wired.fire("refresh-endpoints");

    // Then no refresh is published, so the picker cannot be re-armed behind
    // a gate that already refused to open it.
    assert!(
        !result
            .message_names
            .iter()
            .any(|name| name.ends_with("RefreshEndpointPickerEntries")),
        "an alloy must not be able to trigger a fetch the gate forbids, got {:?}",
        result.message_names
    );
}

#[rstest::rstest]
#[case::single_openrouter(
    ModelSelection::Single("openrouter/anthropic/claude-sonnet-4.5".to_owned()),
    true
)]
#[case::single_direct(
    ModelSelection::Single("anthropic/claude-sonnet-4-5".to_owned()),
    true
)]
#[case::alloy(ModelSelection::Alloy {
    models: vec!["openrouter/a".to_owned()],
    strategy: AlloyStrategy::RoundRobin { index: 0 },
}, false)]
fn routing_requires_a_single_model(#[case] model: ModelSelection, #[case] expected: bool) {
    // Given a session on a model of a given shape.
    // When asking whether the endpoint picker may serve it.
    // Then the answer is the model's shape, not its provider.
    assert_eq!(
        crate::endpoint_picker_actions::may_route_through_endpoint(&model),
        expected
    );
}

// ── Rows and filtering ──────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn open_shows_the_rows_it_was_given() {
    // Given the slice wired with a single model and a three-row list.
    let wired = Wired::new().await;
    wired.set_single_model();
    wired.open();
    wired.with_entries(sample_entries());

    // When reading what the filter shows.
    let visible = wired.visible();

    // Then all three rows are presented, sentinel first.
    assert_eq!(
        visible,
        vec!["auto-route", "Acme", "Borealis"],
        "the picker must present every row it was given"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn typing_in_the_filter_narrows_the_rows() {
    // Given the picker showing three rows.
    let wired = Wired::new().await;
    wired.set_single_model();
    wired.open();
    wired.with_entries(sample_entries());

    // When typing "acme" into the filter.
    wired.type_text("acme");

    // Then only the matching row remains.
    assert_eq!(
        wired.visible(),
        vec!["Acme"],
        "the filter must narrow the rows, case-insensitively"
    );
    // And the filter text is what was typed.
    assert_eq!(wired.filter(), "acme");
}

#[rstest::rstest]
#[tokio::test]
async fn the_filter_also_matches_an_endpoints_tag() {
    // Given the picker showing two upstream rows.
    let wired = Wired::new().await;
    wired.set_single_model();
    wired.open();
    wired.with_entries(sample_entries());

    // When typing a tag rather than a provider name.
    wired.type_text("eu-west");

    // Then the row is found by its tag, so both halves of the label are
    // searchable.
    assert_eq!(wired.visible(), vec!["Borealis"]);
}

#[rstest::rstest]
#[tokio::test]
async fn backspace_widens_the_filter_again() {
    // Given the filter narrowed to a single row.
    let wired = Wired::new().await;
    wired.set_single_model();
    wired.open();
    wired.with_entries(sample_entries());
    wired.type_text("ac");

    // When backspacing once.
    wired.edit(&EditIntent::DeleteBackward);

    // Then the filter text lost its last character.
    assert_eq!(
        wired.filter(),
        "a",
        "backspace must delete the last character"
    );
    // And the result widened: "a" reaches a substring of all three names,
    // where "ac" reached only one.
    assert_eq!(
        wired.visible().len(),
        3,
        "a shorter filter must match more rows, not fewer"
    );
}

// ── Navigation ──────────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn moving_down_moves_the_highlight() {
    // Given the picker showing three rows with the first highlighted.
    let wired = Wired::new().await;
    wired.set_single_model();
    wired.open();
    wired.with_entries(sample_entries());
    assert_eq!(wired.highlighted(), 0);

    // When pressing down.
    wired.fire("move-endpoint-picker-down");

    // Then the highlight follows to the second row.
    assert_eq!(wired.highlighted(), 1);
}

#[rstest::rstest]
#[tokio::test]
async fn moving_up_from_the_first_row_stays_put() {
    // Given the picker with its highlight on the first row.
    let wired = Wired::new().await;
    wired.set_single_model();
    wired.open();
    wired.with_entries(sample_entries());

    // When pressing up.
    wired.fire("move-endpoint-picker-up");

    // Then the highlight does not wrap or go negative.
    assert_eq!(
        wired.highlighted(),
        0,
        "the highlight must clamp at the top rather than wrap"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn paging_down_moves_by_the_measured_window() {
    // Given a picker with many rows and a measured viewport of five.
    let wired = Wired::new().await;
    wired.set_single_model();
    wired.open();
    let many: Vec<EndpointEntry> = (0..40)
        .map(|i| entry(&format!("tag{i}"), &format!("p{i}"), false))
        .collect();
    wired.with_entries(many);
    wired.cell().update(|picker| picker.results_viewport = 5);

    // When paging down.
    wired.fire("page-endpoint-picker-down");

    // Then the highlight advanced by roughly half the window, not one row.
    let moved = wired.highlighted();
    assert!(
        moved > 1,
        "page-down must move by the viewport, not one row; moved to {moved}"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn the_render_pass_publishes_the_measured_viewport() {
    // Given the picker open with rows.
    let wired = Wired::new().await;
    wired.set_single_model();
    wired.open();
    wired.with_entries(sample_entries());

    // When drawing a frame.
    let _ = wired.draw();

    // Then the cell carries the measured row count, so the navigation keys
    // page by a real window rather than a hardcoded constant.
    let viewport = wired.cell().read().results_viewport;
    assert!(
        viewport > 0,
        "the render pass must publish a non-zero viewport, got {viewport}"
    );
}

// ── Confirming ──────────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn confirming_publishes_a_pin_for_the_highlighted_row() {
    // Given the picker open with the second row highlighted.
    let wired = Wired::new().await;
    wired.set_single_model();
    wired.open();
    wired.with_entries(sample_entries());
    wired.fire("move-endpoint-picker-down");

    // When confirming.
    let result = wired.fire("confirm-endpoint-picker");

    // Then a pin is published for the active session's model.
    let pin = Wired::published_pin(result).expect("confirming must publish a pin command");
    assert_eq!(pin.model, "openrouter/anthropic/claude-sonnet-4.5");
    assert_eq!(pin.tag.as_deref(), Some("us-east"));
}

#[rstest::rstest]
#[tokio::test]
async fn confirming_closes_the_picker() {
    // Given the picker open with rows.
    let wired = Wired::new().await;
    wired.set_single_model();
    wired.open();
    wired.with_entries(sample_entries());

    // When confirming.
    let result = wired.fire("confirm-endpoint-picker");

    // Then the picker closes.
    assert_eq!(
        result.scope_signal,
        Some(ScopeSignal::PopIf(endpoint_picker_scope())),
        "confirming must close the picker"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn confirming_the_auto_route_sentinel_publishes_a_removal() {
    // Given the picker open with the auto-route sentinel highlighted.
    let wired = Wired::new().await;
    wired.set_single_model();
    wired.open();
    wired.with_entries(sample_entries());

    // When confirming.
    let result = wired.fire("confirm-endpoint-picker");

    // Then the published command carries no tag, so the row is removed
    // rather than a blank tag written.
    let pin = Wired::published_pin(result).expect("confirming must publish a pin command");
    assert_eq!(pin.model, "openrouter/anthropic/claude-sonnet-4.5");
    assert!(
        pin.tag.is_none(),
        "the auto-route sentinel must remove the pin rather than pin a blank tag"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn cancelling_closes_without_pinning() {
    // Given the picker open over three rows.
    let wired = Wired::new().await;
    wired.set_single_model();
    wired.open();
    wired.with_entries(sample_entries());

    // When cancelling.
    let result = wired.fire("cancel-endpoint-picker");

    // Then the picker closes without publishing a pin.
    assert_eq!(
        result.scope_signal,
        Some(ScopeSignal::PopIf(endpoint_picker_scope()))
    );
    assert!(
        Wired::published_pin(result).is_none(),
        "cancelling must not change the pin"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn confirming_an_empty_menu_pins_nothing() {
    // Given the picker open but with no rows loaded yet.
    let wired = Wired::new().await;
    wired.set_single_model();
    wired.open();

    // When confirming.
    let result = wired.fire("confirm-endpoint-picker");

    // Then no pin command is published, so a mistimed Enter cannot clear a pin.
    assert!(Wired::published_pin(result).is_none());
}

// ── Refreshing ──────────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn refresh_clears_the_stale_rows() {
    // Given the picker showing a previously fetched list.
    let wired = Wired::new().await;
    wired.set_single_model();
    wired.open();
    wired.with_entries(sample_entries());
    assert_eq!(wired.visible().len(), 3);

    // When forcing a refresh.
    wired.fire("refresh-endpoints");

    // Then the rows are cleared, so the menu does not present a stale list as
    // though it were current while the fetch runs.
    assert!(
        wired.visible().is_empty(),
        "a forced refresh must not leave the previous list on screen"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn refresh_publishes_the_forced_command() {
    // Given the picker open on a single model.
    let wired = Wired::new().await;
    wired.set_single_model();
    wired.open();

    // When forcing a refresh.
    let result = wired.fire("refresh-endpoints");

    // Then the cache-bypassing command is published.
    assert!(
        result
            .message_names
            .iter()
            .any(|name| name.ends_with("RefreshEndpointPickerEntries")),
        "ctrl-r must publish the forced-refresh command, got {:?}",
        result.message_names
    );
}

// ── Ctrl-C ──────────────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn clearing_the_filter_leaves_the_picker_open() {
    // Given the picker open with text in its filter.
    let wired = Wired::new().await;
    wired.set_single_model();
    wired.open();
    wired.with_entries(sample_entries());
    wired.type_text("acme");

    // When pressing ctrl-c.
    let result = wired.fire("clear-filter-or-leave-endpoint-picker");

    // Then the filter is emptied and the picker stays open.
    assert_eq!(wired.filter(), "", "ctrl-c must clear the filter text");
    assert_eq!(
        wired.visible().len(),
        3,
        "clearing the filter must restore every row"
    );
    assert_eq!(
        result.scope_signal, None,
        "ctrl-c must not close the picker while there is a filter to clear"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn clearing_an_empty_filter_closes_the_picker() {
    // Given the picker open with an empty filter.
    let wired = Wired::new().await;
    wired.set_single_model();
    wired.open();
    wired.with_entries(sample_entries());

    // When pressing ctrl-c.
    let result = wired.fire("clear-filter-or-leave-endpoint-picker");

    // Then there is nothing left to clear, so the key leaves.
    assert_eq!(
        result.scope_signal,
        Some(ScopeSignal::PopIf(endpoint_picker_scope())),
        "ctrl-c on an already-empty filter must close the picker"
    );
}

// ── Rendering ───────────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn the_picker_draws_a_row_per_endpoint() {
    // Given the picker showing three rows.
    let wired = Wired::new().await;
    wired.set_single_model();
    wired.open();
    wired.with_entries(sample_entries());

    // When drawing a frame.
    let frame = wired.draw();

    // Then each row's provider name appears.
    for name in ["auto-route", "Acme", "Borealis"] {
        assert!(
            frame.iter().any(|row| row.contains(name)),
            "the rendered frame must show {name}"
        );
    }
}

#[rstest::rstest]
#[tokio::test]
async fn the_status_line_shows_the_pinned_upstream() {
    // Given a picker whose rows mark the first upstream as active.
    let wired = Wired::new().await;
    wired.set_single_model();
    wired.open();
    wired.with_entries(vec![
        entry("", "auto-route", false),
        entry("us-east", "Acme", true),
        entry("eu-west", "Borealis", false),
    ]);

    // When drawing a frame.
    let frame = wired.draw();

    // Then the status line names the pinned upstream, not auto-route.
    assert!(
        frame.iter().any(|row| row.contains("Routing: Acme")),
        "the status line must report the pinned upstream; frame: {frame:?}"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn the_status_line_reports_a_fetch_in_flight() {
    // Given the picker open, which flags its fetch.
    let wired = Wired::new().await;
    wired.set_single_model();
    wired.open();
    wired.with_entries(sample_entries());

    // When drawing a frame.
    let frame = wired.draw();

    // Then the line says a fetch is running.
    assert!(
        frame.iter().any(|row| row.contains("fetching")),
        "the status line must show the fetch is in flight; frame: {frame:?}"
    );
}

/// The endpoint menu draws its cost/detail pane.
///
/// The renderer used the plain `SelectionWidget`, which has no pane to draw in,
/// so every row rendered bare and the routing tag, uptime, quantization, and
/// pricing were never shown — even though `endpoint_preview` existed and was
/// correct. Nothing asserted the pane's content, only the rows.
#[rstest::rstest]
#[tokio::test]
async fn endpoint_picker_draws_the_detail_pane() {
    // Given an open picker with a highlighted endpoint carrying metadata.
    let wired = Wired::new().await;
    wired.with_entries(vec![entry("anthropic/claude-sonnet-4", "Anthropic", true)]);

    // When the popup is drawn.
    let rows = wired.draw();
    let text = rows.join("\n");

    // Then the pane shows the endpoint's details, not just its name.
    assert!(
        text.contains("Anthropic"),
        "the detail pane must show the provider: {text}"
    );
    for field in ["Uptime", "Quantization", "Prompt price", "Completion price"] {
        assert!(
            text.contains(field),
            "the detail pane must show {field}: {text}"
        );
    }
}

/// The detail pane scrolls on its own keys, separate from list paging.
#[rstest::rstest]
#[tokio::test]
async fn endpoint_picker_scrolls_the_detail_pane() {
    // Given an open picker.
    let wired = Wired::new().await;
    wired.with_entries(vec![entry("anthropic/claude-sonnet-4", "Anthropic", true)]);

    // When the pane is scrolled down then back up past the top.
    wired.fire("endpoint-preview-down");
    assert_eq!(wired.cell().read().preview_scroll, 1);
    wired.fire("endpoint-preview-up");
    wired.fire("endpoint-preview-up");

    // Then it stops at the top rather than wrapping or underflowing.
    assert_eq!(wired.cell().read().preview_scroll, 0);
}
