//! Model-market slice composition tests.
//!
//! These drive the slice exactly as composition wires it — the real
//! catalog-style registration, the real `activate`, the real route rows
//! and input hook — so a wiring defect fails here instead of at a user's
//! keypress. The actors are NOT spawned (they belong to T2/T3): every
//! test seeds the data cell by hand where the table needs rows.

#![allow(clippy::expect_used, clippy::panic, reason = "test code")]

use jinn_kernel::IntentHandler;
use jinn_kernel::KernelIntent;
use jinn_model_market::market_ui_scope;
use jinn_model_market_msg::MarketDataState;
use jinn_model_market_msg::MarketRow;
use jinn_model_market_msg::MarketUiState;
use jinn_model_market_msg::SortColumn;
use jinn_slices::DynamicIntent;
use jinn_slices::KeyRoutes;
use jinn_slices::PublishSink;
use jinn_slices::RouteResult;
use jinn_slices::Slices;
use jinn_slices::cell::TypedCell;
use jinn_slices::empty_config_layer;
use jinn_slices::route::BindSite;
use ratatui::buffer::Cell;

/// The slice wired exactly as composition wires it: catalog-seeded
/// cells, real activation, real rows and hook.
struct Wired {
    slices: Slices,
    routes: KeyRoutes,
    state: jinn_kernel::AppState,
}

impl Wired {
    /// Builds the slice with the model-market popup registered.
    fn new() -> Self {
        let slices = Slices::new();
        let mut viewport = jinn_slices::view::Viewport::new();
        let overlay_views = jinn_slices::OverlayViews::new();
        let routes = KeyRoutes::new();
        // The actors are T2/T3 territory; an empty system is fine —
        // no test here publishes to an actor.
        let system = trouper::system::ActorSystem::new(trouper::system::SystemConfig::production());
        let mut host =
            jinn_slices::SliceHost::new(&slices, &mut viewport, &overlay_views, &routes, &system);
        // The cell catalog's role: production registers every slot before
        // any slice activates, so the harness does the same — including
        // the scope-focus slot the state's scope stack writes through.
        host.register_cell(
            jinn_model_market::market_data_slot(),
            MarketDataState::default(),
        )
        .expect("fresh Slices never has the data slot registered");
        host.register_cell(
            jinn_model_market::market_ui_slot(),
            MarketUiState::default(),
        )
        .expect("fresh Slices never has the UI slot registered");
        host.register_cell(
            jinn_slices::scope_focus_slot(),
            jinn_slices::ScopeFocusState::default(),
        )
        .expect("fresh Slices never has the scope-focus slot registered");
        jinn_model_market::activate(&mut host);
        let state = {
            let state = jinn_kernel::AppState::default();
            state.frontend.attach_slices(slices.clone());
            state
        };
        Self {
            slices,
            routes,
            state,
        }
    }

    /// The UI cell handle.
    fn ui(&self) -> TypedCell<MarketUiState> {
        self.slices
            .reader(&jinn_model_market::market_ui_slot())
            .expect("the registration in `new` minted the UI cell")
    }

    /// The data cell handle.
    fn data(&self) -> TypedCell<MarketDataState> {
        self.slices
            .reader(&jinn_model_market::market_data_slot())
            .expect("the registration in `new` minted the data cell")
    }

    /// Dispatches a dynamic intent through the real route table — the
    /// handler path production dispatch uses.
    fn handle(&mut self, intent: DynamicIntent) -> RouteResult {
        IntentHandler::handle(
            &KernelIntent::Dynamic(intent),
            &mut self.state,
            &self.slices,
            &self.routes,
            empty_config_layer(),
        )
    }

    /// Presses a bound popup key: mints the dynamic intent the keymap
    /// would mint for the row's action, then dispatches it. The popup's
    /// scope must be focused first (the opener pushes it, or the test
    /// focuses it directly) — the hook consult resolves by focus.
    fn press(&mut self, action: &'static str, display: &'static str) -> RouteResult {
        self.handle(DynamicIntent::new(market_ui_scope(), action, display))
    }
}

/// One priced listing row for a test table.
fn row(
    id: &str,
    author: &str,
    slug: &str,
    prompt: f64,
    completion: f64,
    cache: Option<f64>,
) -> MarketRow {
    MarketRow {
        id: id.to_owned(),
        author: author.to_owned(),
        slug: slug.to_owned(),
        variant: None,
        prompt_price: prompt,
        completion_price: completion,
        cache_read_price: cache,
        has_overrides: false,
        context_length: Some(128_000),
        eci: None,
        eci_ci: None,
        is_free: false,
    }
}

/// A two-row table: cheap-then-expensive in prompt price.
fn seeded(wired: &mut Wired) {
    wired.data().update(|d| {
        d.rows = vec![
            row(
                "openrouter/a/small",
                "a",
                "small",
                0.000_001,
                0.000_002,
                Some(0.000_000_1),
            ),
            row("openrouter/b/big", "b", "big", 0.000_003, 0.000_006, None),
        ];
    });
}

/// Decodes the first published message of type `M` out of a route
/// result, draining the closures with a payload-collecting sink — the
/// typed twin of the `message_names` assertions. Publish closures are
/// `FnOnce`, so the result is consumed.
fn first_message<M>(result: &mut RouteResult) -> Option<M>
where
    M: jinn_slices::BusMessage
        + trouper::schema::Schema
        + serde::Serialize
        + serde::de::DeserializeOwned
        + Clone
        + Send
        + Sync
        + trouper::envelope::PayloadValue,
{
    let full_name = std::any::type_name::<M>();
    let mut found: Option<serde_json::Value> = None;
    for closure in std::mem::take(&mut result.messages) {
        if found.is_some() {
            break;
        }
        let cell: std::cell::RefCell<Option<serde_json::Value>> = std::cell::RefCell::new(None);
        struct Collect<'a> {
            want: &'a str,
            found: &'a std::cell::RefCell<Option<serde_json::Value>>,
        }
        impl PublishSink for Collect<'_> {
            fn publish_schema(
                &self,
                _schema_id: trouper::schema::SchemaId,
                payload: serde_json::Value,
                name: &'static str,
            ) {
                // `RouteResult::push_message` records the full type
                // name; match on the suffix so the test reads the same
                // message the `message_names` assertions see.
                if name.ends_with(self.want) {
                    *self.found.borrow_mut() = Some(payload);
                }
            }
        }
        closure(&Collect {
            want: full_name.rsplit("::").next().unwrap_or(full_name),
            found: &cell,
        });
        found = cell.into_inner();
    }
    found.and_then(|payload| serde_json::from_value::<M>(payload).ok())
}

/// The editing intent the composition keymap's hook catch-all mints for
/// a printable character in the popup's scope.
fn edit_intent(action: &'static str, ch: char) -> DynamicIntent {
    DynamicIntent::with_bytes(
        market_ui_scope(),
        action,
        action,
        ch.to_string().into_bytes(),
    )
}

/// The dynamic intent a bracketed paste travels as — the shape the TUI
/// mints and the kernel's paste branch routes by focus.
fn paste_intent(text: &str) -> DynamicIntent {
    DynamicIntent::with_bytes(
        market_ui_scope(),
        jinn_chat_input_msg::PASTE_TEXT_ACTION,
        "paste text",
        text.as_bytes().to_vec(),
    )
}

// ── Wiring ──────────────────────────────────────────────────────────────

#[rstest::rstest]
fn cells_resolve_through_real_activation() {
    // Given the slice wired over a registry seeded the way the catalog
    // seeds it.
    let wired = Wired::new();

    // When resolving both cells.
    let ui = wired.ui();
    let data = wired.data();

    // Then both hold usable defaults.
    assert_eq!(ui.read().sort_col, SortColumn::Proj);
    assert!(data.read().rows.is_empty());
}

#[rstest::rstest]
fn every_advertised_key_binds_in_the_popup_scope() {
    // Given the wired slice's route rows, its registered hook scope,
    // and the footer's advertised bindings.
    let wired = Wired::new();
    let bound: Vec<&str> = wired
        .routes
        .rows()
        .iter()
        .filter(|r| r.feature == "model-market")
        .map(|r| r.key)
        .collect();
    let hook_owned = wired
        .routes
        .input_hook_scopes()
        .contains(&market_ui_scope());

    // When every advertised key is checked.
    for (key, _) in jinn_model_market::routes::MARKET_BINDINGS {
        // Non-character keys must be route rows.
        if !is_char_key(key) {
            assert!(
                bound.contains(key),
                "footer advertises {key} but no route row binds it"
            );
            continue;
        }
        // Character keys must be hook cases: a char key cannot be a
        // row in an input-hook scope, or route dispatch would steal it
        // from filter typing.
        assert!(
            hook_owned,
            "footer advertises {key} but the popup scope has no input hook"
        );
    }
}

/// Whether a footer key is a single printable character (no `<...>`
/// wrapper, no modifier).
fn is_char_key(key: &str) -> bool {
    key.len() == 1 && key.chars().next().is_some_and(|c| !c.is_control())
}

#[rstest::rstest]
fn every_market_char_key_dispatches_through_the_composition_keymap() {
    // Given the composed slice's keymap state at the popup's dynamic
    // scope — the resolution path production dispatch uses.
    let wired = Wired::new();
    let mut keymap = jinn_tui::keymap::init();
    jinn_tui::keymap_gen::bind_route_rows(&wired.routes, &mut keymap);
    let mut state =
        ratatui_which_key::WhichKeyState::new(keymap, jinn_tui::Scope::Dynamic(market_ui_scope()));

    // When every advertised character key is pressed.
    for (key, _) in jinn_model_market::routes::MARKET_BINDINGS {
        if !is_char_key(key) {
            continue;
        }
        let ch = key.chars().next().unwrap_or_default();
        let intent = state.handle_key(key_event_char(ch));

        // Then it resolves — to the insert-char edit intent the hook
        // consult consumes, never an unbound drop.
        let Some(jinn_kernel::KernelIntent::Dynamic(dynamic)) = intent else {
            panic!("footer char key {key} resolved to no intent in the popup scope");
        };
        assert_eq!(
            dynamic.action, "insert-char",
            "char key {key} must dispatch as an insert-char edit intent"
        );
        assert_eq!(
            dynamic.slice,
            market_ui_scope(),
            "the insert-char intent must target the popup's scope"
        );
    }
}

#[rstest::rstest]
fn every_market_non_char_key_resolves_to_its_row() {
    // Given the same composed keymap state at the popup's scope.
    let wired = Wired::new();
    let mut keymap = jinn_tui::keymap::init();
    jinn_tui::keymap_gen::bind_route_rows(&wired.routes, &mut keymap);
    let mut state =
        ratatui_which_key::WhichKeyState::new(keymap, jinn_tui::Scope::Dynamic(market_ui_scope()));

    // When every advertised non-character key is pressed.
    for (key, _) in jinn_model_market::routes::MARKET_BINDINGS {
        if is_char_key(key) {
            continue;
        }
        let intent = state.handle_key(key_event_notation(key));

        // Then it resolves to the dynamic intent its route row names.
        let Some(jinn_kernel::KernelIntent::Dynamic(dynamic)) = intent else {
            panic!("footer key {key} resolved to no intent in the popup scope");
        };
        assert_eq!(
            dynamic.slice,
            market_ui_scope(),
            "key {key} must dispatch within the popup's scope"
        );
    }
}

/// A bare character key event.
fn key_event_char(c: char) -> jinn_kernel::KeyEvent {
    jinn_kernel::KeyEvent {
        key: jinn_kernel::Key::Char(c),
        modifiers: jinn_kernel::Modifiers::none(),
    }
}

/// Parses a footer notation (`<enter>`, `<pgdn>`) into a key event.
#[expect(
    clippy::expect_used,
    reason = "a footer notation that fails to parse is a table bug this test names"
)]
fn key_event_notation(notation: &str) -> jinn_kernel::KeyEvent {
    let events = ratatui_which_key::parse_key_sequence::<jinn_kernel::KeyEvent>(
        notation,
        &key_event_char('\\'),
    );
    events
        .into_iter()
        .next()
        .expect("a single-key notation yields one event")
}

#[rstest::rstest]
fn j_and_k_move_the_highlight() {
    // Given a seeded table, table focused.
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");
    seeded(&mut wired);

    // When `j` arrives as the insert-char intent the keymap's catch-all
    // mints (route rows cannot own char keys in a hook scope).
    wired.handle(edit_intent("insert-char", 'j'));

    // Then the highlight moved down.
    assert_eq!(wired.ui().read().selection, 1);
    // And `k` moved it back up.
    wired.handle(edit_intent("insert-char", 'k'));
    assert_eq!(wired.ui().read().selection, 0);
    // And `k` at the top clamps.
    wired.handle(edit_intent("insert-char", 'k'));
    assert_eq!(wired.ui().read().selection, 0);
}

#[rstest::rstest]
fn h_l_s_m_while_the_filter_is_focused_type_instead_of_firing() {
    // Given an open popup with the filter focused, PROJ ascending,
    // first mix preset.
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");
    wired.ui().update(|s| s.filter_focused = true);

    // When "hlsm" is typed.
    for ch in ['h', 'l', 's', 'm'] {
        wired.handle(edit_intent("insert-char", ch));
    }

    // Then every letter landed in the filter text.
    assert_eq!(wired.ui().read().filter, "hlsm");
    // And no sort/mix action fired: still PROJ, ascending, first
    // preset.
    assert_eq!(wired.ui().read().sort_col, SortColumn::Proj);
    assert!(!wired.ui().read().sort_desc);
    assert_eq!(wired.ui().read().mix_index, 0);
    // And the filter is still focused.
    assert!(wired.ui().read().filter_focused);
}

#[rstest::rstest]
fn h_l_s_m_fire_when_the_table_is_focused() {
    // Given an open popup, table focused.
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");

    // When `l`, `h`, `s`, and `m` each arrive.
    wired.handle(edit_intent("insert-char", 'l'));
    wired.handle(edit_intent("insert-char", 'h'));
    wired.handle(edit_intent("insert-char", 's'));
    wired.handle(edit_intent("insert-char", 'm'));

    // Then the sort is still PROJ (l then h round-trips) but the
    // direction flipped and the mix advanced.
    assert_eq!(wired.ui().read().sort_col, SortColumn::Proj);
    assert!(wired.ui().read().sort_desc);
    assert_eq!(wired.ui().read().mix_index, 1);
    // And none of the letters typed into the filter.
    assert!(wired.ui().read().filter.is_empty());
}

#[rstest::rstest]
fn the_opener_binds_in_the_normal_scope() {
    // Given the wired slice's rows.
    let wired = Wired::new();
    let rows = wired.routes.rows();
    let opener = rows
        .iter()
        .find(|r| r.route_id.as_str() == "model-market:open")
        .expect("the slice attaches its opener row");

    // Then it binds in the Normal static scope, not its own — a key
    // that opens a scope cannot live inside it.
    assert_eq!(opener.key, "<leader>so");
    assert_eq!(opener.site, BindSite::StaticScopes(&["Normal"]));
}

// ── Open + ESC semantics ────────────────────────────────────────────────

#[rstest::rstest]
fn open_resets_ui_state_and_pushes_the_scope() {
    // Given UI state carried over from a previous open.
    let mut wired = Wired::new();
    wired.ui().update(|s| {
        s.filter = "sonnet".to_owned();
        s.filter_focused = true;
        s.sort_col = SortColumn::Eci;
        s.sort_desc = true;
        s.mix_index = 2;
    });

    // When the opener fires.
    let result = wired.press("open-model-market", "model market");

    // Then the UI cell is back to the opener defaults: table focused,
    // filter empty, PROJ ascending, first preset.
    let ui = wired.ui();
    assert!(ui.read().filter.is_empty());
    assert!(!ui.read().filter_focused);
    assert_eq!(ui.read().sort_col, SortColumn::Proj);
    assert!(!ui.read().sort_desc);
    assert_eq!(ui.read().mix_index, 0);
    // And the handler applied the push (the signal is consumed by
    // `apply_scope_signal`, so the stack is the observable) and the
    // fetch was requested.
    assert_eq!(
        wired.state.frontend.scope(),
        jinn_slices::FocusScope::Dynamic(market_ui_scope())
    );
    assert!(result.message_names.contains(&"FetchModels"));
}

#[rstest::rstest]
fn esc_in_table_mode_pops_the_scope() {
    // Given an open popup with the table focused.
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");

    // When ESC fires with the filter unfocused.
    wired.press("close-model-market", "close");

    // Then the scope popped (the signal is consumed by the handler).
    assert_eq!(wired.state.frontend.scope(), jinn_slices::FocusScope::Input);
}

#[rstest::rstest]
fn esc_in_filter_mode_returns_focus_to_the_table() {
    // Given an open popup with the filter focused.
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");
    wired.ui().update(|s| s.filter_focused = true);

    // When ESC fires.
    wired.press("close-model-market", "close");

    // Then the filter hands focus back and the popup stays open.
    assert!(!wired.ui().read().filter_focused);
    assert_eq!(
        wired.state.frontend.scope(),
        jinn_slices::FocusScope::Dynamic(market_ui_scope())
    );
}

// ── Filter focus (the hook) ─────────────────────────────────────────────

#[rstest::rstest]
fn f_focuses_the_filter_from_the_table() {
    // Given an open popup, table focused.
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");

    // When `f` arrives as the editing intent the keymap's catch-all
    // mints.
    wired.handle(edit_intent("insert-char", 'f'));

    // Then the filter is focused and the character was not inserted.
    let ui = wired.ui();
    assert!(ui.read().filter_focused);
    assert!(ui.read().filter.is_empty());
}

#[rstest::rstest]
fn i_also_focuses_the_filter_from_the_table() {
    // Given an open popup, table focused.
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");

    // When `i` arrives.
    wired.handle(edit_intent("insert-char", 'i'));

    // Then the filter is focused.
    assert!(wired.ui().read().filter_focused);
}

#[rstest::rstest]
fn typing_while_filter_focused_inserts_into_the_filter() {
    // Given an open popup with the filter focused.
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");
    wired.ui().update(|s| s.filter_focused = true);

    // When a character arrives.
    wired.handle(edit_intent("insert-char", 's'));

    // Then it landed in the filter text, and the selection reset to the
    // top of the narrowed list.
    let ui = wired.ui();
    assert_eq!(ui.read().filter, "s");
    assert_eq!(ui.read().selection, 0);
}

#[rstest::rstest]
fn f_types_into_a_focused_filter_instead_of_refocusing() {
    // Given an open popup with the filter focused.
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");
    wired.ui().update(|s| s.filter_focused = true);

    // When `f` arrives.
    wired.handle(edit_intent("insert-char", 'f'));

    // Then the character was inserted — the focus flip did not re-fire.
    assert_eq!(wired.ui().read().filter, "f");
}

#[rstest::rstest]
fn unclaimed_characters_fall_through_the_hook() {
    // Given an open popup, table focused.
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");

    // When an unclaimed character arrives (`z`).
    wired.handle(edit_intent("insert-char", 'z'));

    // Then nothing changed: the hook did not claim it.
    let ui = wired.ui();
    assert!(!ui.read().filter_focused);
    assert!(ui.read().filter.is_empty());
}

#[rstest::rstest]
fn backspace_edits_a_focused_filter() {
    // Given a focused filter holding "son".
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");
    wired.ui().update(|s| {
        s.filter_focused = true;
        s.filter = "son".to_owned();
    });

    // When delete-backward arrives.
    wired.handle(edit_intent("delete-backward", '\u{8}'));

    // Then the filter shrank by the one character.
    assert_eq!(wired.ui().read().filter, "so");
}

#[rstest::rstest]
fn paste_into_a_focused_filter_appends_the_text() {
    // Given an open popup with the filter focused and holding "so".
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");
    wired.ui().update(|s| {
        s.filter_focused = true;
        s.filter = "so".to_owned();
    });

    // When a bracketed paste arrives — the intent the kernel's paste
    // branch routes to the focused scope's input hook.
    wired.handle(paste_intent("nnet"));

    // Then the pasted text landed in the filter and the selection reset.
    let ui = wired.ui();
    assert_eq!(ui.read().filter, "sonnet");
    assert_eq!(ui.read().selection, 0);
}

#[rstest::rstest]
fn paste_while_the_table_is_focused_is_declined() {
    // Given an open popup, table focused.
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");

    // When a bracketed paste arrives.
    wired.handle(paste_intent("sonnet"));

    // Then the hook declined it: the filter is untouched and stays
    // unfocused — the text belongs to the next surface, not the table.
    let ui = wired.ui();
    assert!(!ui.read().filter_focused);
    assert!(ui.read().filter.is_empty());
}

// ── ENTER triple ────────────────────────────────────────────────────────

#[rstest::rstest]
fn enter_publishes_the_confirm_triple() {
    // Given a seeded table with the first row highlighted.
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");
    seeded(&mut wired);

    // When ENTER fires.
    let result = wired.press("confirm-model-market", "switch model");

    // Then all three legs of the triple are present, and the handler
    // popped the popup's scope (the signal is consumed).
    assert_eq!(
        result.message_names,
        vec!["ProviderSwitch", "UpdateAppState"]
    );
    assert_eq!(wired.state.frontend.scope(), jinn_slices::FocusScope::Input);
}

#[rstest::rstest]
fn enter_carries_the_highlighted_row_and_this_session() {
    // Given a seeded table with the first row highlighted.
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");
    seeded(&mut wired);

    // When ENTER fires.
    let mut result = wired.press("confirm-model-market", "switch model");

    // Then the ProviderSwitch leg decodes with the highlighted row's id
    // and this session's id.
    let switch: jinn_provider_selection_msg::ProviderSwitch =
        first_message(&mut result).expect("ProviderSwitch publishes");
    let session_id = jinn_slices::route::SliceActionState::active_session_id(&wired.state);
    assert_eq!(switch.session_id, session_id);
    assert_eq!(
        switch.provider_id,
        jinn_core_types::model_selection::ModelSelection::Single("openrouter/a/small".to_owned())
    );
}

#[rstest::rstest]
fn enter_without_a_highlighted_row_is_a_no_op() {
    // Given an open popup over an empty table.
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");

    // When ENTER fires.
    let result = wired.press("confirm-model-market", "switch model");

    // Then nothing publishes and the popup stays open.
    assert!(result.messages.is_empty());
    assert_eq!(
        wired.state.frontend.scope(),
        jinn_slices::FocusScope::Dynamic(market_ui_scope())
    );
}

// ── Sort / mix / navigation ─────────────────────────────────────────────

#[rstest::rstest]
fn sort_column_cycles_l_through_all_six_columns() {
    // Given an open popup (PROJ by default).
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");

    // When `l` fires five times (a hook case — the keymap's
    // insert-char catch-all owns it in this scope).
    for _ in 0..5 {
        wired.handle(edit_intent("insert-char", 'l'));
    }

    // Then the cycle walked PROJ → IN → OUT → CACHE → ECI → MODEL.
    assert_eq!(wired.ui().read().sort_col, SortColumn::Model);
    // And `h` walks back.
    wired.handle(edit_intent("insert-char", 'h'));
    assert_eq!(wired.ui().read().sort_col, SortColumn::Eci);
}

#[rstest::rstest]
fn sort_direction_toggles_s() {
    // Given an open popup (ascending by default).
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");

    // When `s` fires (a hook case).
    wired.handle(edit_intent("insert-char", 's'));

    // Then the direction flipped.
    assert!(wired.ui().read().sort_desc);
    // And again restores it.
    wired.handle(edit_intent("insert-char", 's'));
    assert!(!wired.ui().read().sort_desc);
}

#[rstest::rstest]
fn mix_cycles_m_through_three_presets() {
    // Given an open popup (first preset).
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");

    // When `m` fires twice (a hook case).
    wired.handle(edit_intent("insert-char", 'm'));
    let second = wired.ui().read().active_mix();
    wired.handle(edit_intent("insert-char", 'm'));

    // Then the presets advanced in order.
    assert_eq!(second, jinn_model_market_msg::MIX_PRESETS[1]);
    assert_eq!(
        wired.ui().read().active_mix(),
        jinn_model_market_msg::MIX_PRESETS[2]
    );
}

#[rstest::rstest]
fn navigation_moves_and_clamps_at_the_table_end() {
    // Given a seeded two-row table.
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");
    seeded(&mut wired);

    // When moving down twice.
    wired.press("move-model-market-down", "down");
    wired.press("move-model-market-down", "down");

    // Then the highlight sits on the last row.
    assert_eq!(wired.ui().read().selection, 1);
    // And a third move clamps instead of overflowing.
    wired.press("move-model-market-down", "down");
    assert_eq!(wired.ui().read().selection, 1);
    // And moving up steps back.
    wired.press("move-model-market-up", "up");
    assert_eq!(wired.ui().read().selection, 0);
    // And moving up at the top clamps.
    wired.press("move-model-market-up", "up");
    assert_eq!(wired.ui().read().selection, 0);
}

#[rstest::rstest]
fn j_clamps_against_the_live_filtered_count_not_the_viewport() {
    // Given a two-row table filtered down to both rows, with the
    // measured viewport (10) larger than the filtered list.
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");
    seeded(&mut wired);
    wired.ui().update(|s| s.results_viewport = 10);
    assert_eq!(wired.ui().read().results_viewport, 10);

    // When `j` fires five times — past the list's end.
    for _ in 0..5 {
        wired.handle(edit_intent("insert-char", 'j'));
    }

    // Then the selection clamped at the last filtered row.
    assert_eq!(wired.ui().read().selection, 1);

    // And the highlight renders at that clamped position.
    let buffer = render_to_buffer(&wired, 120, 40);
    let theme = jinn_theme::default_theme();
    let highlighted = buffer
        .content
        .iter()
        .filter(|c| c.bg == theme.picker_selected_bg)
        .count();
    assert!(
        highlighted > 0,
        "the clamped selection must still render highlighted"
    );
}

#[rstest::rstest]
fn navigation_into_an_unfetched_row_publishes_a_debounced_fetch() {
    // Given a seeded table with no detail loaded.
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");
    seeded(&mut wired);

    // When moving down.
    let mut result = wired.press("move-model-market-down", "down");

    // Then a FetchEndpoints rides the result, stamped with the UI
    // cell's incremented sequence.
    let fetches = first_message::<jinn_model_market_msg::FetchEndpoints>(&mut result)
        .expect("navigation into an unfetched row fetches");
    assert_eq!(fetches.model_id, "openrouter/b/big");
    assert_eq!(fetches.seq, wired.ui().read().fetch_seq);
}

#[rstest::rstest]
fn navigation_with_detail_already_present_publishes_nothing() {
    // Given a seeded table whose second row's detail is already loaded.
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");
    seeded(&mut wired);
    wired.data().update(|d| {
        d.detail = Some(jinn_model_market_msg::MarketDetail {
            model_id: "openrouter/b/big".to_owned(),
            endpoints: vec![],
        });
    });

    // When moving down.
    let result = wired.press("move-model-market-down", "down");

    // Then no fetch publishes.
    assert!(result.messages.is_empty());
}

// ── Help toggle (the hook's `?` case) ───────────────────────────────────

#[rstest::rstest]
fn question_mark_toggles_the_help_card() {
    // Given an open popup, table focused.
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");

    // When `?` arrives.
    wired.handle(edit_intent("insert-char", '?'));

    // Then the help card is showing.
    assert!(wired.ui().read().help_visible);
    // And `?` again hides it.
    wired.handle(edit_intent("insert-char", '?'));
    assert!(!wired.ui().read().help_visible);
}

#[rstest::rstest]
fn question_mark_types_while_the_filter_is_focused() {
    // Given an open popup with the filter focused.
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");
    wired.ui().update(|s| s.filter_focused = true);

    // When `?` arrives.
    wired.handle(edit_intent("insert-char", '?'));

    // Then it typed into the filter instead of toggling the card.
    assert_eq!(wired.ui().read().filter, "?");
    assert!(!wired.ui().read().help_visible);
}

// ── Refresh ─────────────────────────────────────────────────────────────

#[rstest::rstest]
fn ctrl_r_publishes_refresh_all() {
    // Given an open popup.
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");

    // When <c-r> fires.
    let result = wired.press("refresh-model-market", "refresh all");

    // Then the drop-everything command publishes.
    assert_eq!(result.message_names, vec!["RefreshAll"]);
}

// ── Render: the stamp clock (§5.3) ──────────────────────────────────────

#[rstest::rstest]
fn clock_renders_from_stamp_without_network() {
    // Given a data cell stamped two minutes in the past.
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");
    seeded(&mut wired);
    wired.data().update(|d| {
        d.models_fetched_at = Some(jiff::Timestamp::now() - jiff::Span::new().minutes(2));
    });

    // When the popup renders against a test backend (no bus, no HTTP).
    let theme = jinn_theme::default_theme();
    let area = ratatui::layout::Rect::new(0, 0, 120, 40);
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(area.width, area.height))
            .expect("test terminal");
    let popup = jinn_model_market::view::market_overlay_rect(&area).expect("frame fits the popup");
    terminal
        .draw(|frame| {
            let facts = jinn_slices::RenderFacts::new(theme.clone(), &wired.slices);
            jinn_model_market::view::render_market_popup(frame, popup, &facts);
        })
        .expect("draw");

    // Then the status line carries the stamp-derived age, computed from
    // the cell's stamp — no network involved.
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(
        text.contains("models updated 2m ago"),
        "status line must render the stamp age, got: {text:?}"
    );
    // And the seeded rows are on the table.
    assert!(
        text.contains("a/small"),
        "the seeded row must render, got: {text:?}"
    );
}

// ── Render: table shape, footer, filter bar ─────────────────────────────

/// Renders the current popup frame and returns the buffer's text plus a
/// per-row extractor. `Wired` must already have its cells seeded.
fn render_to_buffer(wired: &Wired, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let theme = jinn_theme::default_theme();
    let area = ratatui::layout::Rect::new(0, 0, width, height);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
        .expect("test terminal");
    let popup = jinn_model_market::view::market_overlay_rect(&area).expect("frame fits the popup");
    terminal
        .draw(|frame| {
            let facts = jinn_slices::RenderFacts::new(theme.clone(), &wired.slices);
            jinn_model_market::view::render_market_popup(frame, popup, &facts);
        })
        .expect("draw");
    terminal.backend().buffer().clone()
}

/// A rendered row's text: the buffer row's cells joined.
fn row_text(buffer: &ratatui::buffer::Buffer, y: u16, width: u16) -> String {
    (0..width)
        .filter_map(|x| buffer.cell((x, y)).map(Cell::symbol))
        .collect()
}

#[rstest::rstest]
fn table_headers_align_over_their_data_columns() {
    // Given a seeded table with distinct prices per column, table
    // focused.
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");
    seeded(&mut wired);
    wired.data().update(|d| {
        if let Some(first) = d.rows.first_mut() {
            first.has_overrides = true;
            first.is_free = false;
        }
    });

    // When the popup renders.
    let buffer = render_to_buffer(&wired, 120, 40);
    let popup =
        jinn_model_market::view::market_overlay_rect(&ratatui::layout::Rect::new(0, 0, 120, 40))
            .expect("frame fits the popup");

    // Then the header row carries every column name in order.
    let header = row_text(&buffer, popup.y + 1, popup.width);
    for name in ["PROJ", "IN/M", "OUT/M", "CACHE/M", "ECI", "MODEL", "FLAGS"] {
        assert!(
            header.contains(name),
            "header row must name {name}, got: {header:?}"
        );
    }
    // And the first data row shows each of the first listing's values
    // on the same line: PROJ cents, $/M prices, the model, and the FLAGS
    // glyph for the pricing override.
    let data_row = row_text(&buffer, popup.y + 2, popup.width);
    assert!(
        data_row.contains("\u{a2}"),
        "PROJ cell renders cents, got: {data_row:?}"
    );
    assert!(
        data_row.contains("a/small"),
        "MODEL cell renders the name, got: {data_row:?}"
    );
    assert!(
        data_row.contains('\u{2020}'),
        "FLAGS cell renders the override dagger, got: {data_row:?}"
    );
    // And a row with no cache price shows the em-dash under CACHE/M.
    assert!(
        data_row.contains("\u{2014}"),
        "CACHE cell renders an em-dash when unpublished, got: {data_row:?}"
    );
}

#[rstest::rstest]
fn footer_renders_the_advertised_keys() {
    // Given an open popup.
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");
    seeded(&mut wired);

    // When the popup renders.
    let area = ratatui::layout::Rect::new(0, 0, 120, 40);
    let popup = jinn_model_market::view::market_overlay_rect(&area).expect("frame fits the popup");
    let buffer = render_to_buffer(&wired, 120, 40);

    // Then the popup's last inner row carries the footer: keys and
    // labels from MARKET_BINDINGS.
    let footer = row_text(&buffer, popup.y + popup.height - 2, popup.width);
    assert!(
        footer.contains("<enter>"),
        "footer shows the confirm key, got: {footer:?}"
    );
    assert!(
        footer.contains("<esc>"),
        "footer shows the close key, got: {footer:?}"
    );
    assert!(
        footer.contains("<c-r>"),
        "footer shows the refresh key, got: {footer:?}"
    );
    assert!(
        footer.contains("<pgdn>"),
        "footer shows the page key, got: {footer:?}"
    );
}

#[rstest::rstest]
fn focused_filter_shows_a_visible_filter_bar() {
    // Given an open popup with the filter focused and text typed.
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");
    wired.ui().update(|s| {
        s.filter_focused = true;
        s.filter = "son".to_owned();
    });
    seeded(&mut wired);

    // When the popup renders.
    let area = ratatui::layout::Rect::new(0, 0, 120, 40);
    let popup = jinn_model_market::view::market_overlay_rect(&area).expect("frame fits the popup");
    let buffer = render_to_buffer(&wired, 120, 40);
    let title = row_text(&buffer, popup.y, popup.width);

    // Then the block title is the filter bar: the filter text and a
    // cursor block.
    assert!(
        title.contains("filter: son"),
        "focused title shows the filter, got: {title:?}"
    );
    assert!(
        title.contains('\u{2588}'),
        "focused title shows a cursor, got: {title:?}"
    );
    // And an unfocused popup shows the plain title instead.
    wired.ui().update(|s| s.filter_focused = false);
    let buffer = render_to_buffer(&wired, 120, 40);
    let title = row_text(&buffer, popup.y, popup.width);
    assert!(
        title.contains("Model Market"),
        "unfocused title is plain, got: {title:?}"
    );
    assert!(
        !title.contains("filter:"),
        "unfocused title hides the bar, got: {title:?}"
    );
}

#[rstest::rstest]
fn status_line_shows_the_mix_and_its_m_hint() {
    // Given an open popup over a seeded table.
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");
    seeded(&mut wired);

    // When the popup renders.
    let area = ratatui::layout::Rect::new(0, 0, 120, 40);
    let popup = jinn_model_market::view::market_overlay_rect(&area).expect("frame fits the popup");
    let buffer = render_to_buffer(&wired, 120, 40);
    let status = row_text(&buffer, popup.y + popup.height - 3, popup.width);

    // Then the status line carries the mix summary and the adjust hint.
    assert!(
        status.contains("mix: 80k in"),
        "status shows the mix, got: {status:?}"
    );
    assert!(
        status.contains("[m] adjust"),
        "status shows the m hint, got: {status:?}"
    );
    // And cycling the mix changes what the next frame shows.
    wired.handle(edit_intent("insert-char", 'm'));
    let buffer = render_to_buffer(&wired, 120, 40);
    let status = row_text(&buffer, popup.y + popup.height - 3, popup.width);
    assert!(
        status.contains("mix: 150k in"),
        "status reflects the cycled mix, got: {status:?}"
    );
}

#[rstest::rstest]
fn filter_matches_are_highlighted_in_the_model_column() {
    // Given an open popup filtered down to one row.
    let mut wired = Wired::new();
    wired.press("open-model-market", "model market");
    seeded(&mut wired);
    wired.ui().update(|s| s.filter = "small".to_owned());

    // When the popup renders.
    let area = ratatui::layout::Rect::new(0, 0, 120, 40);
    let popup = jinn_model_market::view::market_overlay_rect(&area).expect("frame fits the popup");
    let buffer = render_to_buffer(&wired, 120, 40);

    // Then only the matching row renders.
    let popup_rows: Vec<String> = (popup.y + 2..popup.y + 5)
        .map(|y| row_text(&buffer, y, popup.width))
        .collect();
    assert!(
        popup_rows.iter().any(|r| r.contains("a/small")),
        "the matching row renders: {popup_rows:?}"
    );
    assert!(
        !popup_rows.iter().any(|r| r.contains("b/big")),
        "the non-matching row is filtered out: {popup_rows:?}"
    );
    // And the matched glyphs carry the shared highlighter's background:
    // scanning the row's cells, at least one cell in the MODEL column
    // carries the theme's highlight bg (the filter's match spans).
    let theme = jinn_theme::default_theme();
    let y = (popup.y + 2..popup.y + popup.height - 3)
        .find(|y| row_text(&buffer, *y, popup.width).contains("a/small"))
        .expect("the matching row renders");
    let highlighted = (popup.x..popup.x + popup.width)
        .filter(|x| {
            buffer
                .cell((*x, y))
                .is_some_and(|c| c.style().bg == Some(theme.picker_highlight_bg))
        })
        .count();
    assert!(
        highlighted >= 1,
        "the filter's match spans carry the highlight bg, found {highlighted}"
    );
    // And the matched glyphs are the filter's letters ("s","m","a","l","l").
    let glyphs: String = (popup.x..popup.x + popup.width)
        .filter(|x| {
            buffer
                .cell((*x, y))
                .is_some_and(|c| c.style().bg == Some(theme.picker_highlight_bg))
        })
        .filter_map(|x| buffer.cell((x, y)).map(Cell::symbol))
        .collect();
    assert_eq!(glyphs, "small");
}
