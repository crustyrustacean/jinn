//! Slice-composition integration tests.
//!
//! These exercise the composed system — real slice activation over the
//! kernel's registries, real route rows, real cells and actors — the
//! layer no individual crate can test in isolation. They live here (the
//! root crate's `tests/`) so `just check` and IDE analysis never compile
//! slice crates into the kernel or the tui crate graph.
//!
//! Coverage split:
//! - tui unit tests: synthetic/slice-shaped inputs only
//! - slice crate tests: each slice's own row shape and behavior
//! - these tests: the composition of both (keys resolve across the
//!   built-in keymap plus every slice's rows; rendering against real
//!   slice cells; actors applying routed messages).
#![allow(clippy::expect_used, clippy::panic, reason = "test code")]

mod common;

use common::{composed_keymap, composition_routes, test_app, wait_for};
use jinn_dashboard::dashboard_scope;
use jinn_domain::common::slices::TypedCell;
use jinn_domain::{Intent, Key, KeyEvent, Modifiers};
use jinn_quake_bar::quake_scope;
use jinn_tui::Scope;
use ratatui_which_key::NodeResult;

fn plain(ch: char) -> KeyEvent {
    KeyEvent {
        key: Key::Char(ch),
        modifiers: Modifiers::none(),
    }
}

// ---------------------------------------------------------------------------
// Composed keymap resolution
// ---------------------------------------------------------------------------

/// The composed `gdc` sequence resolves to discord's to-thread action:
/// the slice's row survives the merge into the composed keymap.
#[rstest::rstest]
#[test]
fn gdc_resolves_to_discord_to_thread_in_the_composed_keymap() {
    // Given the composed keymap (built-in bindings + every slice's rows).
    let keymap = composed_keymap();

    // When navigating the gdc sequence in the Normal scope.
    let result = keymap
        .navigate(&[plain('g'), plain('d'), plain('c')], &Scope::Normal)
        .expect("gdc path exists in the composed keymap");

    // Then it resolves to a dynamic intent for discord's to-thread action.
    let NodeResult::Leaf { action } = result else {
        panic!("gdc must be a leaf, got {result:?}");
    };
    let Intent::Dynamic(dynamic) = action else {
        panic!("gdc must resolve to a dynamic intent, got {action:?}");
    };
    assert_eq!(dynamic.slice.key(), "discord:actions");
    assert_eq!(dynamic.action, "to-thread");
}

/// The `gd` prefix under `g` derives a group labeled "discord" (the
/// feature label), while the root `g` keeps its hardcoded "general" label.
#[rstest::rstest]
#[test]
fn gd_prefix_derives_the_discord_group_label() {
    // Given the composed keymap.
    let keymap = composed_keymap();

    // When listing the children under the `g` prefix in Normal scope.
    let g_children = keymap
        .children_at_path(&[plain('g')], &Scope::Normal)
        .expect("g group bindings");

    // Then the `d` child is described as the discord group.
    assert!(
        g_children
            .iter()
            .any(|b| b.key == plain('d') && b.description == "discord"),
        "gd group should be derived with the discord label, got {g_children:?}"
    );
    // And the root `g` keeps its hardcoded "general" description.
    let root = keymap
        .children_at_path(&[], &Scope::Normal)
        .expect("root bindings");
    assert!(
        root.iter()
            .any(|b| b.key == plain('g') && b.description == "general"),
        "root g should keep the general label, got {root:?}"
    );
}

/// The discord row does not pierce typing: no `g` branch exists in the
/// Input scope even with every slice's rows attached.
#[rstest::rstest]
#[test]
fn gdc_is_absent_from_the_input_scope() {
    // Given the composed keymap.
    let keymap = composed_keymap();

    // When navigating the g prefix in the Input scope.
    let result = keymap.navigate(&[plain('g')], &Scope::Input);

    // Then nothing resolves — typing is untouched by slice rows.
    assert!(
        result.is_none(),
        "gdc must not bind in Input; got {result:?}"
    );
}

/// The quake `<M-\`>` toggle: open from static scopes, close inside the
/// quake's own dynamic scope (specific-scope-wins).
#[rstest::rstest]
#[test]
fn quake_backtick_toggles_open_in_normal_and_close_in_quake_scope() {
    // Given the composed keymap in the Normal scope.
    let keymap = composed_keymap();
    let alt_backtick = KeyEvent {
        key: Key::Char('`'),
        modifiers: Modifiers {
            ctrl: false,
            alt: true,
            shift: false,
        },
    };

    // When pressing <M-`> in Normal scope.
    let intent = {
        let mut wk = jinn_tui::app::WhichKeyInstance::new(keymap.clone(), Scope::Normal);
        wk.handle_key(alt_backtick.clone())
    };

    // Then it resolves to the quake open action.
    let intent = intent.expect("<M-`> must open the quake bar from Normal");
    assert!(
        matches!(&intent, Intent::Dynamic(d) if d.action == "open"),
        "expected the quake open action, got {intent:?}"
    );

    // And when pressing <M-`> in the quake's own scope, it resolves to
    // close — making <M-`> a toggle (specific-scope-wins over the opener).
    let mut wk = jinn_tui::app::WhichKeyInstance::new(keymap, Scope::Dynamic(quake_scope()));
    let intent = wk
        .handle_key(alt_backtick)
        .expect("<M-`> must resolve in QuakeBar");
    assert!(
        matches!(&intent, Intent::Dynamic(d) if d.action == "close"),
        "expected the quake close action, got {intent:?}"
    );
}

/// ESC resolves to the quake close action inside the quake scope.
#[rstest::rstest]
#[test]
fn esc_fires_quake_close_in_quake_scope() {
    // Given the composed keymap in the quake's dynamic scope.
    let keymap = composed_keymap();
    let mut wk = jinn_tui::app::WhichKeyInstance::new(keymap, Scope::Dynamic(quake_scope()));

    // When pressing ESC.
    let esc = KeyEvent {
        key: Key::Esc,
        modifiers: Modifiers::none(),
    };
    let intent = wk.handle_key(esc);

    // Then it resolves to the quake close action (which pops the scope).
    let intent = intent.expect("ESC in QuakeBar scope must fire an intent");
    assert!(
        matches!(&intent, Intent::Dynamic(d) if d.action == "close"),
        "ESC must resolve to the quake close action; got {intent:?}"
    );
}

/// A printable char in the quake input-hook scope synthesizes InsertChar:
/// the hook only sees intents the keymap emits.
#[rstest::rstest]
#[test]
fn printable_char_synthesizes_insert_char_in_quake_hook_scope() {
    // Given the composed keymap in the quake's dynamic scope.
    let keymap = composed_keymap();
    let mut wk = jinn_tui::app::WhichKeyInstance::new(keymap, Scope::Dynamic(quake_scope()));

    // When pressing a plain printable char.
    let key_x = KeyEvent {
        key: Key::Char('x'),
        modifiers: Modifiers::none(),
    };
    let intent = wk.handle_key(key_x);

    // Then which-key synthesizes the generic editing intent for the hook
    // scopes (the handler's hook consult routes it to the slice writer).
    assert!(
        matches!(intent, Some(Intent::InsertChar { ch: 'x' })),
        "printable char must synthesize InsertChar for the slice input hook; got {intent:?}"
    );
}

/// PageUp resolves to the quake scroll-up action (so the log scrolls).
#[rstest::rstest]
#[test]
fn pgup_fires_quake_scroll_up_in_quake_scope() {
    // Given the composed keymap in the quake's dynamic scope.
    let keymap = composed_keymap();
    let mut wk = jinn_tui::app::WhichKeyInstance::new(keymap, Scope::Dynamic(quake_scope()));

    // When pressing PageUp.
    let pgup = KeyEvent {
        key: Key::PageUp,
        modifiers: Modifiers::none(),
    };
    let intent = wk.handle_key(pgup);

    // Then it resolves to the quake scroll-up action.
    let intent = intent.expect("PageUp in QuakeBar scope must fire an intent");
    assert!(
        matches!(&intent, Intent::Dynamic(d) if d.action == "scroll-up"),
        "PageUp must resolve to the quake scroll-up action; got {intent:?}"
    );
}

/// The composed keymap carries the terminal-overlay toggle in the
/// dashboard's dynamic scope: registered slice scopes get the per-scope
/// chrome too.
#[rstest::rstest]
#[test]
fn alt_t_resolves_in_the_dashboard_dynamic_scope() {
    // Given the composed keymap in the dashboard's dynamic scope.
    let keymap = composed_keymap();
    let mut wk = jinn_tui::app::WhichKeyInstance::new(keymap, Scope::Dynamic(dashboard_scope()));

    // When pressing <M-t>.
    let alt_t = KeyEvent {
        key: Key::Char('t'),
        modifiers: Modifiers {
            ctrl: false,
            alt: true,
            shift: false,
        },
    };
    let intent = wk.handle_key(alt_t);

    // Then the terminal overlay toggle fires.
    assert!(
        matches!(
            intent,
            Some(Intent::ToggleTerminalOverlay { session_id: None })
        ),
        "dashboard scope: expected ToggleTerminalOverlay, got {intent:?}"
    );
}

// ---------------------------------------------------------------------------
// Leak guard
// ---------------------------------------------------------------------------

/// Kernel chat-history bindings never leak into the dashboard scope:
/// the slice's scope carries only its own rows.
#[rstest::rstest]
#[test]
fn dashboard_scope_has_no_chathistory_or_sidebar_bindings() {
    // Given the composed keymap.
    let keymap = composed_keymap();

    // When listing the dashboard scope's bindings.
    let groups = keymap.bindings_for_scope(Scope::Dynamic(dashboard_scope()));
    let all_desc: Vec<String> = groups
        .iter()
        .flat_map(|g| g.bindings.iter().map(|b| b.description.clone()))
        .collect();

    // Then no ChatHistory group descriptions appear.
    assert!(
        !all_desc
            .iter()
            .any(|d| d.contains("next") || d.contains("previous")),
        "ChatHistory groups leaked into Dashboard: {all_desc:?}"
    );
}

// ---------------------------------------------------------------------------
// Composition seam
// ---------------------------------------------------------------------------

/// The row seam carries every slice's rows: dashboard, quake-bar, and
/// discord all attached (the precondition the keymap tests rely on).
#[rstest::rstest]
#[test]
fn composition_sees_rows_from_every_slice() {
    // Given the composed route table.
    let routes = composition_routes();

    // When listing the dynamic scopes it knows about.
    let scopes = jinn_tui::keymap_gen::dynamic_scopes(&routes);

    // Then every slice's scope is present.
    assert!(
        scopes.iter().any(|s| *s == dashboard_scope()),
        "dashboard scope missing from composed routes"
    );
    assert!(
        scopes.iter().any(|s| *s == quake_scope()),
        "quake-bar scope missing from composed routes"
    );
    assert!(
        scopes.iter().any(|s| *s == jinn_discord::discord_scope()),
        "discord scope missing from composed routes"
    );
}

// ---------------------------------------------------------------------------
// End-to-end: keypress → keymap → router → actor → cell
// ---------------------------------------------------------------------------

/// E2E: the j keypress routes through the composed keymap to the
/// dashboard actor, which applies the selection move to the slice cell.
#[rstest::rstest]
#[tokio::test]
async fn j_keypress_routes_to_dashboard_actor_and_moves_selection() {
    // Given a wired app: the harness runs `dashboard::activate`, which
    // spawns THE dashboard actor subscribed to the bus.
    let mut app = test_app().await;
    app.core
        .state
        .write_test_no_cap()
        .frontend
        .scope_stack
        .swap_base(jinn_domain::FocusScope::Dynamic(
            jinn_dashboard::dashboard_scope(),
        ));
    let slot = jinn_dashboard::dashboard_slot();
    let cell: TypedCell<jinn_dashboard::DashboardState> =
        app.services.slices.reader(&slot).expect("cell");
    cell.update(|d| {
        for i in 0..3 {
            d.mark_running(format!("actor-{i}"), None);
        }
    });
    app.which_key
        .set_scope(Scope::Dynamic(jinn_dashboard::dashboard_scope()));

    // When the j key resolves through the keymap and routes like the run loop.
    let protocol_key = {
        use crossterm::event::{KeyCode, KeyEvent as XKeyEvent, KeyModifiers};
        jinn_tui::convert::from_crossterm(XKeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("j converts")
    };
    let before = cell.read().selected_index();
    let intent = app
        .which_key
        .handle_key(protocol_key)
        .expect("j resolves in Dashboard scope");
    app.route_intent(intent);
    wait_for("the dashboard actor to move the selection", || {
        cell.read().selected_index() == before + 1
    })
    .await;

    // Then the actor applied the move to the slice.
    assert_eq!(
        cell.read().selected_index(),
        before + 1,
        "j moves selection via the routed message"
    );
}

// ---------------------------------------------------------------------------
// Rendering against real slice cells
// ---------------------------------------------------------------------------

/// Writes into the dashboard slice cell through the app registry.
fn write_dashboard(app: &jinn_tui::TuiApp, f: impl FnOnce(&mut jinn_dashboard::DashboardState)) {
    let cell: TypedCell<jinn_dashboard::DashboardState> = app
        .services
        .slices
        .reader(&jinn_dashboard::dashboard_slot())
        .expect("harness registers the dashboard slot");
    cell.update(f);
}

/// Collects the entire terminal buffer into a single string.
fn buffer_string(terminal: &ratatui::Terminal<ratatui::backend::TestBackend>) -> String {
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect()
}

fn render_app(app: &mut jinn_tui::TuiApp) -> ratatui::Terminal<ratatui::backend::TestBackend> {
    let (mut terminal, _area) = jinn_testutil::setup_term(80, 24);
    terminal
        .draw(|frame| app.render(frame))
        .expect("render succeeds");
    terminal
}

async fn dashboard_app() -> jinn_tui::TuiApp {
    let app = test_app().await;
    app.core
        .state
        .write_test_no_cap()
        .frontend
        .scope_stack
        .swap_base(jinn_domain::FocusScope::Dynamic(
            jinn_dashboard::dashboard_scope(),
        ));
    app
}

/// The registered dashboard tab renders highlighted in its own scope.
#[rstest::rstest]
#[tokio::test]
async fn registered_tab_is_highlighted_in_its_scope() {
    // Given a composed app whose base scope is the registered dashboard tab.
    let mut app = dashboard_app().await;
    let (mut terminal, _area) = jinn_testutil::setup_term(80, 24);
    terminal.draw(|frame| app.render(frame)).expect("render");

    // Then the dashboard tab cell has an active background (non-Reset).
    let layout = jinn_tui::render::app_layout::AppLayout::new(
        ratatui::layout::Rect::new(0, 0, 80, 24),
        1,
        12,
        30,
    );
    let buffer = terminal.backend().buffer();
    // " Chat " (6 cols) + separator space (1) = 7 cols offset.
    let dash_x = layout.tab_bar.x + 1 + " Chat ".len() as u16 + 1;
    let cell = buffer
        .cell((dash_x, layout.tab_bar.y))
        .expect("dashboard tab cell");
    assert_ne!(
        cell.bg,
        ratatui::style::Color::Reset,
        "dashboard tab should be highlighted in Dashboard scope"
    );
}

/// The dashboard tab stays highlighted when another overlay opens (the
/// tab bar follows the base scope, not the top of the stack).
#[rstest::rstest]
#[tokio::test]
async fn registered_tab_stays_highlighted_when_another_overlay_opens() {
    // Given a composed app in the dashboard scope with a quake overlay pushed.
    let mut app = dashboard_app().await;
    app.core
        .state
        .write_test_no_cap()
        .frontend
        .scope_stack
        .push(jinn_domain::FocusScope::Dynamic(
            jinn_slices::SliceScopeId::new("quake-bar", "bar"),
        ));
    let (mut terminal, _area) = jinn_testutil::setup_term(80, 24);
    terminal.draw(|frame| app.render(frame)).expect("render");

    // Then the dashboard tab is still highlighted (uses base scope, not top).
    let layout = jinn_tui::render::app_layout::AppLayout::new(
        ratatui::layout::Rect::new(0, 0, 80, 24),
        1,
        12,
        30,
    );
    let buffer = terminal.backend().buffer();
    let dash_x = layout.tab_bar.x + 1 + " Chat ".len() as u16 + 1;
    let chat_cell = buffer
        .cell((layout.tab_bar.x + 1, layout.tab_bar.y))
        .expect("chat tab cell");
    let dash_cell = buffer
        .cell((dash_x, layout.tab_bar.y))
        .expect("dashboard tab cell");
    assert_eq!(
        chat_cell.bg,
        ratatui::style::Color::Reset,
        "chat tab should NOT be highlighted when base is Dashboard"
    );
    assert_ne!(
        dash_cell.bg,
        ratatui::style::Color::Reset,
        "dashboard tab should stay highlighted when an overlay is open"
    );
}

/// The dashboard tab shows the actor name and lifecycle column.
#[rstest::rstest]
#[tokio::test]
async fn dashboard_tab_shows_actor_name_and_lifecycle() {
    // Given a composed app with a dashboard cell holding one running actor.
    let mut app = dashboard_app().await;
    write_dashboard(&app, |d| {
        d.mark_running("discord", Some("Discord bot".to_owned()));
    });

    // When rendering.
    let terminal = render_app(&mut app);

    // Then the buffer contains "discord".
    let buf_str = buffer_string(&terminal);
    assert!(buf_str.contains("discord"), "dashboard should show name");
    // And the lifecycle column reads "Running".
    assert!(
        buf_str.contains("Running"),
        "dashboard should show lifecycle"
    );
}

/// The dashboard tab shows a per-actor status message.
#[rstest::rstest]
#[tokio::test]
async fn dashboard_tab_shows_status_message_for_discord() {
    // Given a composed app with discord in a connected state.
    let mut app = dashboard_app().await;
    write_dashboard(&app, |d| {
        d.mark_running("discord", None);
        d.set_status_message("discord", Some("Connected".to_owned()));
    });

    // When rendering.
    let terminal = render_app(&mut app);

    // Then the buffer contains "Connected".
    let buf_str = buffer_string(&terminal);
    assert!(
        buf_str.contains("Connected"),
        "dashboard should show status message"
    );
}

/// An empty dashboard cell renders the placeholder.
#[rstest::rstest]
#[tokio::test]
async fn dashboard_tab_shows_empty_placeholder_when_no_actors() {
    // Given a composed app with an empty dashboard cell.
    let mut app = dashboard_app().await;
    write_dashboard(&app, jinn_dashboard::DashboardState::clear);

    // When rendering.
    let terminal = render_app(&mut app);

    // Then the buffer contains the placeholder.
    let buf_str = buffer_string(&terminal);
    assert!(
        buf_str.contains("No services"),
        "empty dashboard should show placeholder"
    );
}

/// The selected dashboard row carries the selection marker.
#[rstest::rstest]
#[tokio::test]
async fn dashboard_tab_shows_selection_marker_on_selected_entry() {
    // Given a composed app with two actors, the second selected.
    let mut app = dashboard_app().await;
    write_dashboard(&app, |d| {
        d.mark_running("alpha", None);
        d.mark_running("beta", None);
        d.select_next(); // select beta (index 1)
    });

    // When rendering.
    let terminal = render_app(&mut app);

    // Then the buffer contains the selection marker ▸.
    let buf_str = buffer_string(&terminal);
    assert!(buf_str.contains('▸'), "selected entry should have marker");
}

/// The dashboard tab draws no em-dash separator between name and
/// description.
#[rstest::rstest]
#[tokio::test]
async fn dashboard_tab_has_no_em_dash_separator() {
    // Given a composed app with an actor that has a description.
    let mut app = dashboard_app().await;
    write_dashboard(&app, |d| {
        d.mark_running("discord", Some("Discord bot".to_owned()));
    });

    // When rendering.
    let terminal = render_app(&mut app);

    // Then the buffer contains no em-dash characters.
    let buf_str = buffer_string(&terminal);
    assert!(
        !buf_str.contains('\u{2014}'),
        "dashboard should not contain em-dashes"
    );
}
